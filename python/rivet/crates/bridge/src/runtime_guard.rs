//! Turning "there is no simulation running" into a Python error rather
//! than a Rust panic.
//!
//! Every entry point that touches the runtime goes through here, so a
//! testbench imported outside a simulator (an editor, a linter, `pytest`
//! collecting) fails with something a person can act on.

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rivet_core::runtime;

pub fn ensure_running() -> PyResult<()> {
    if runtime::is_initialised() {
        Ok(())
    } else {
        Err(PyRuntimeError::new_err(
            "no simulation is running — this only works inside a test run by `rivet run`, \
             or against the mock simulator from `rivet.mock`",
        ))
    }
}

pub fn precision() -> PyResult<i32> {
    ensure_running()?;
    Ok(runtime::precision())
}

pub fn now() -> PyResult<u64> {
    ensure_running()?;
    Ok(runtime::now())
}

/// Run `f`, turning a panic from the runtime into a Python exception.
///
/// The Rust API panics on a bad read or an illegal write because in Rust
/// that is a bug in the testbench and the backtrace is the diagnosis.
/// Across the boundary a panic would unwind into CPython, so it is caught
/// and re-raised with the same message.
pub fn guard<R>(f: impl FnOnce() -> R) -> PyResult<R> {
    let was = QUIET.with(|q| q.replace(true));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    QUIET.with(|q| q.set(was));
    match r {
        Ok(v) => Ok(v),
        Err(p) => Err(PyRuntimeError::new_err(runtime::panic_message(&p))),
    }
}

thread_local! {
    /// Set while [`guard`] is running, so the panic hook below knows the
    /// panic is about to become a Python exception.
    static QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Stop a panic that is on its way to becoming a Python exception from
/// also printing a Rust backtrace.
///
/// `dut.en.set(1)` in the ReadOnly phase is a mistake in the testbench,
/// and the testbench is in Python: the useful output is the Python
/// traceback and the message, not a hundred frames of interpreter
/// internals. The message is preserved — [`guard`] re-raises it — and the
/// backtrace is still there at debug level.
pub fn install_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if QUIET.with(|q| q.get()) {
                log::debug!("{info}");
            } else {
                previous(info);
            }
        }));
    });
}
