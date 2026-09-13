//! Transaction traces compared against committed golden files.
//!
//! ```ignore
//! let mut trace = Trace::new("axi_writes");
//! trace.record(format!("W {addr:#x} = {data:#x}"));   // stamped with sim time
//! // ...
//! assert_trace!(trace);   // compares with golden/<test>__axi_writes.trace
//! ```
//!
//! The golden directory is `RIVET_GOLDEN_DIR` (the CLI sets it to
//! `<crate>/golden`); the file is `<module>_<test>[@<param set>]__<name>.trace`.
//! Time stamps count from the trace's creation, so a test records the same
//! trace whether it runs first, last, or in its own shard. A missing golden file fails the test with the
//! command to create it; `RIVET_UPDATE_GOLDEN=1` rewrites goldens from the
//! current run. On a mismatch the actual trace is written next to the
//! golden as `<name>.actual` and the failure message shows the first
//! differing line with context.

use rivet_core::error::{Error, Result};
use rivet_core::runtime;
use rivet_core::time::{format_time, Unit};
use std::path::PathBuf;

pub struct Trace {
    name: String,
    lines: Vec<String>,
    stamp: bool,
    /// Time stamps are relative to this instant (the trace's creation), so
    /// the same test records the same trace whatever ran before it.
    t0: u64,
}

impl Trace {
    /// A trace whose lines are prefixed with the simulation time.
    pub fn new(name: &str) -> Trace {
        Trace { name: name.into(), lines: Vec::new(), stamp: true, t0: runtime::now() }
    }

    /// A trace without time stamps (for order-only comparisons).
    pub fn unstamped(name: &str) -> Trace {
        Trace { name: name.into(), lines: Vec::new(), stamp: false, t0: 0 }
    }

    pub fn record(&mut self, item: impl std::fmt::Display) {
        if self.stamp {
            let t = format_time(runtime::now().saturating_sub(self.t0), runtime::precision(), Unit::Ns);
            self.lines.push(format!("{t:>12} {item}"));
        } else {
            self.lines.push(item.to_string());
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn text(&self) -> String {
        let mut s = self.lines.join("\n");
        s.push('\n');
        s
    }

    /// Golden file path for this trace under the current test.
    pub fn golden_path(&self) -> Result<PathBuf> {
        let dir = std::env::var("RIVET_GOLDEN_DIR")
            .ok()
            .filter(|d| !d.is_empty())
            .ok_or_else(|| Error::Msg("RIVET_GOLDEN_DIR is not set (rivet run sets it to <crate>/golden)".into()))?;
        let test = rivet_core::log::current_test().unwrap_or_else(|| "no_test".into()).replace("::", "_");
        // Goldens are per parameter set: the design differs.
        let set = rivet_core::test::param_set().map(|s| format!("@{s}")).unwrap_or_default();
        Ok(PathBuf::from(dir).join(format!("{test}{set}__{}.trace", self.name)))
    }

    /// Compare with the golden file (or write it under `RIVET_UPDATE_GOLDEN=1`).
    pub fn check_golden(&self) -> Result<()> {
        let path = self.golden_path()?;
        let actual = self.text();
        if std::env::var("RIVET_UPDATE_GOLDEN").map(|v| v == "1").unwrap_or(false) {
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p).map_err(|e| Error::Msg(format!("cannot create {}: {e}", p.display())))?;
            }
            std::fs::write(&path, &actual).map_err(|e| Error::Msg(format!("cannot write {}: {e}", path.display())))?;
            log::info!("trace {}: golden updated at {}", self.name, path.display());
            return Ok(());
        }
        let expected = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => {
                let actual_path = path.with_extension("trace.actual");
                if let Some(p) = path.parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                let _ = std::fs::write(&actual_path, &actual);
                return Err(Error::Msg(format!(
                    "trace {}: no golden file at {} (actual written to {}; rerun with RIVET_UPDATE_GOLDEN=1 to accept)",
                    self.name,
                    path.display(),
                    actual_path.display()
                )));
            }
        };
        match diff(&expected, &actual) {
            None => {
                log::info!("trace {}: {} line(s) match {}", self.name, self.lines.len(), path.display());
                Ok(())
            }
            Some(d) => {
                let actual_path = path.with_extension("trace.actual");
                let _ = std::fs::write(&actual_path, &actual);
                Err(Error::Msg(format!(
                    "trace {} differs from {}:\n{d}(actual written to {})",
                    self.name,
                    path.display(),
                    actual_path.display()
                )))
            }
        }
    }
}

/// First difference between two texts, with a few lines of context.
pub fn diff(expected: &str, actual: &str) -> Option<String> {
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let first = (0..e.len().max(a.len())).find(|&i| e.get(i) != a.get(i))?;
    let mut out = String::new();
    let start = first.saturating_sub(2);
    for (i, line) in e.iter().enumerate().take(first).skip(start) {
        out.push_str(&format!("   {:>5}  {line}\n", i + 1));
    }
    match (e.get(first), a.get(first)) {
        (Some(x), Some(y)) => {
            out.push_str(&format!("-  {:>5}  {x}\n", first + 1));
            out.push_str(&format!("+  {:>5}  {y}\n", first + 1));
        }
        (Some(x), None) => out.push_str(&format!(
            "-  {:>5}  {x}\n   (actual ends here: {} vs {} lines)\n",
            first + 1,
            a.len(),
            e.len()
        )),
        (None, Some(y)) => out.push_str(&format!(
            "+  {:>5}  {y}\n   (golden ends here: {} vs {} lines)\n",
            first + 1,
            e.len(),
            a.len()
        )),
        (None, None) => unreachable!(),
    }
    Some(out)
}

/// `assert_trace!(trace)` compares a [`Trace`] with its golden file,
/// returning the error from the enclosing test on mismatch.
#[macro_export]
macro_rules! assert_trace {
    ($trace:expr) => {
        $trace.check_golden()?
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_reports_first_change() {
        assert!(diff("a\nb\n", "a\nb\n").is_none());
        let d = diff("a\nb\nc\nd\n", "a\nb\nX\nd\n").unwrap();
        assert!(d.contains("-      3  c"), "{d}");
        assert!(d.contains("+      3  X"), "{d}");
        assert!(d.contains("       1  a"), "{d}");
        let d = diff("a\nb\n", "a\n").unwrap();
        assert!(d.contains("actual ends here: 1 vs 2 lines"), "{d}");
        let d = diff("a\n", "a\nb\n").unwrap();
        assert!(d.contains("golden ends here"), "{d}");
    }
}
