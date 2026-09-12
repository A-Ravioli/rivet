//! A `log` implementation that prefixes simulation time.

use crate::runtime;
use crate::time::{format_time, Unit};
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::io::Write;

struct SimLogger {
    unit: Unit,
}

impl Log for SimLogger {
    fn enabled(&self, _m: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Logging from inside a runtime borrow must not re-enter it.
        let time = match runtime::try_time() {
            Some((now, prec)) => format_time(now, prec, self.unit),
            None => "-".to_string(),
        };
        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };
        let stderr = std::io::stderr();
        let mut out = stderr.lock();
        let _ = writeln!(out, "{:>14} {} {:<24} {}", time, level, record.target(), record.args());
    }

    fn flush(&self) {}
}

/// Install the logger. Level from `RIVET_LOG` (`error`, `warn`, `info`,
/// `debug`, `trace`), default `info`.
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
    let logger: Box<dyn Log> = Box::new(SimLogger { unit });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}
