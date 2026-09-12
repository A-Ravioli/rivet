//! A `log` implementation that prefixes simulation time.
//!
//! - `RIVET_LOG`: level (`error`, `warn`, `info`, `debug`, `trace`, `off`);
//!   default `info`.
//! - `RIVET_LOG_FORMAT`: `text` (default) or `json` (one object per line
//!   with `t` in precision steps, `time`, `level`, `target`, `test`, `msg`).
//! - `RIVET_LOG_TIME_UNIT`: `ns` (default), `ps`, `us`, `step`.
//! - `RIVET_LOG_DIR`: if set, every test also gets its own file
//!   `<dir>/<module>__<name>.log` with the same format, so a failing test's
//!   log can be read without scrolling through the whole regression.

use crate::runtime;
use crate::time::{format_time, Unit};
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;

#[derive(Copy, Clone, PartialEq, Eq)]
enum Format {
    Text,
    Json,
}

struct SimLogger {
    unit: Unit,
    format: Format,
}

struct Current {
    name: String,
    file: Option<std::io::BufWriter<std::fs::File>>,
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
    static LOG_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The test currently running, as `module::name` (for log records).
pub fn current_test() -> Option<String> {
    CURRENT.with(|c| c.borrow().as_ref().map(|t| t.name.clone()))
}

/// Mark the start of a test: records carry its name and, if `RIVET_LOG_DIR`
/// is set, also go to its own file. Called by the regression runner.
pub fn begin_test(module: &str, name: &str) {
    let full = format!("{module}::{name}");
    let file = LOG_DIR.with(|d| {
        d.borrow().as_ref().and_then(|dir| {
            std::fs::create_dir_all(dir).ok()?;
            let path = dir.join(format!("{}__{name}.log", module.replace("::", "_")));
            std::fs::File::create(path).ok().map(std::io::BufWriter::new)
        })
    });
    CURRENT.with(|c| *c.borrow_mut() = Some(Current { name: full, file }));
}

/// End the current test, flushing its log file.
pub fn end_test() {
    CURRENT.with(|c| {
        if let Some(mut cur) = c.borrow_mut().take() {
            if let Some(f) = cur.file.as_mut() {
                let _ = f.flush();
            }
        }
    });
}

/// Set the per-test log directory (the runner reads `RIVET_LOG_DIR`; this
/// is for embedding harnesses).
pub fn set_log_dir(dir: Option<PathBuf>) {
    LOG_DIR.with(|d| *d.borrow_mut() = dir);
}

fn json_escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl SimLogger {
    fn render(&self, record: &Record, test: Option<&str>) -> String {
        // Logging from inside a runtime borrow must not re-enter it.
        let (steps, time) = match runtime::try_time() {
            Some((now, prec)) => (Some(now), format_time(now, prec, self.unit)),
            None => (None, "-".to_string()),
        };
        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };
        match self.format {
            Format::Text => format!("{:>14} {} {:<24} {}", time, level, record.target(), record.args()),
            Format::Json => {
                let mut s = String::with_capacity(128);
                s.push_str("{\"t\":");
                match steps {
                    Some(n) => s.push_str(&n.to_string()),
                    None => s.push_str("null"),
                }
                s.push_str(",\"time\":");
                json_escape(&time, &mut s);
                s.push_str(",\"level\":");
                json_escape(level.trim_end(), &mut s);
                s.push_str(",\"target\":");
                json_escape(record.target(), &mut s);
                s.push_str(",\"test\":");
                match test {
                    Some(t) => json_escape(t, &mut s),
                    None => s.push_str("null"),
                }
                s.push_str(",\"msg\":");
                json_escape(&record.args().to_string(), &mut s);
                s.push('}');
                s
            }
        }
    }
}

impl Log for SimLogger {
    fn enabled(&self, _m: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // The per-test state is thread-local; a record from another thread
        // (the watchdog) simply has no test.
        let line = CURRENT.with(|c| {
            let mut cur = c.try_borrow_mut().ok();
            let cur = cur.as_mut().and_then(|c| c.as_mut());
            let line = self.render(record, cur.as_ref().map(|c| c.name.as_str()));
            if let Some(f) = cur.and_then(|c| c.file.as_mut()) {
                let _ = writeln!(f, "{line}");
            }
            line
        });
        let stderr = std::io::stderr();
        let mut out = stderr.lock();
        let _ = writeln!(out, "{line}");
    }

    fn flush(&self) {
        CURRENT.with(|c| {
            if let Ok(mut cur) = c.try_borrow_mut() {
                if let Some(f) = cur.as_mut().and_then(|c| c.file.as_mut()) {
                    let _ = f.flush();
                }
            }
        });
    }
}

/// Install the logger, reading the `RIVET_LOG*` variables.
pub fn init() {
    let level = match std::env::var("RIVET_LOG").as_deref().map(|s| s.to_ascii_lowercase()).as_deref() {
        Ok("error") => LevelFilter::Error,
        Ok("warn") => LevelFilter::Warn,
        Ok("debug") => LevelFilter::Debug,
        Ok("trace") => LevelFilter::Trace,
        Ok("off") => LevelFilter::Off,
        _ => LevelFilter::Info,
    };
    let unit = match std::env::var("RIVET_LOG_TIME_UNIT").as_deref() {
        Ok("ps") => Unit::Ps,
        Ok("us") => Unit::Us,
        Ok("step") => Unit::Step,
        _ => Unit::Ns,
    };
    let format = match std::env::var("RIVET_LOG_FORMAT").as_deref().map(|s| s.to_ascii_lowercase()).as_deref() {
        Ok("json") => Format::Json,
        _ => Format::Text,
    };
    if let Ok(d) = std::env::var("RIVET_LOG_DIR") {
        if !d.is_empty() {
            set_log_dir(Some(PathBuf::from(d)));
        }
    }
    let logger: Box<dyn Log> = Box::new(SimLogger { unit, format });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escaping() {
        let mut s = String::new();
        json_escape("a\"b\\c\nd\u{1}", &mut s);
        assert_eq!(s, "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn json_record_shape_without_runtime() {
        let l = SimLogger { unit: Unit::Ns, format: Format::Json };
        let line = l.render(
            &log::Record::builder().args(format_args!("hello {}", 1)).level(Level::Warn).target("t").build(),
            Some("m::n"),
        );
        assert_eq!(line, r#"{"t":null,"time":"-","level":"WARN","target":"t","test":"m::n","msg":"hello 1"}"#);
        let l = SimLogger { unit: Unit::Ns, format: Format::Text };
        let line = l.render(
            &log::Record::builder().args(format_args!("hello {}", 1)).level(Level::Warn).target("t").build(),
            None,
        );
        assert!(line.ends_with("WARN  t                        hello 1"));
    }

    #[test]
    fn per_test_files() {
        let dir = std::env::temp_dir().join(format!("rivet-logdir-{}", std::process::id()));
        set_log_dir(Some(dir.clone()));
        begin_test("mod::sub", "case");
        assert_eq!(current_test().as_deref(), Some("mod::sub::case"));
        CURRENT.with(|c| {
            let mut cur = c.borrow_mut();
            let f = cur.as_mut().unwrap().file.as_mut().expect("file opened");
            writeln!(f, "line").unwrap();
        });
        end_test();
        assert!(current_test().is_none());
        let text = std::fs::read_to_string(dir.join("mod_sub__case.log")).unwrap();
        assert_eq!(text, "line\n");
        set_log_dir(None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
