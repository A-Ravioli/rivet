//! The PLI plugin: what the simulator actually loads.
//!
//! The simulator dlopens this library and calls the startup routine in
//! its table. That starts an interpreter with `_rivet` already in the
//! inittab — so the testbench's `import rivet` finds this process's own
//! bindings rather than a wheel that knows nothing about the running
//! simulation — installs the VPI (or VHPI) backend, and replaces the
//! default Rust entry point with one that imports the Python testbench.
//!
//! Build it with exactly one interface:
//!
//! ```sh
//! cargo build --release -p rivet-python-plugin                        # VPI
//! cargo build --release -p rivet-python-plugin --no-default-features --features vhpi
//! ```

use pyo3::prelude::*;

#[cfg(all(feature = "vpi", feature = "vhpi"))]
compile_error!(
    "build the plugin with either `vpi` or `vhpi`, not both: a shared object carrying two \
     PLI startup tables fails to load"
);

#[cfg(not(any(feature = "vpi", feature = "vhpi")))]
compile_error!("the plugin needs one of the `vpi` or `vhpi` features");

/// The bindings, as the interpreter inside the simulator sees them.
#[pymodule]
#[pyo3(name = "_rivet")]
fn rivet_builtin(m: &Bound<'_, PyModule>) -> PyResult<()> {
    rivet_bridge::register_module(m)
}

/// Start the interpreter, with `_rivet` available as a builtin module.
fn start_interpreter() {
    // Must happen before the interpreter starts, so `import _rivet`
    // resolves here and not to a wheel on `sys.path`.
    pyo3::append_to_inittab!(rivet_builtin);
    pyo3::prepare_freethreaded_python();

    // Take the GIL once and keep it. The simulator's thread is the only
    // thread in the process that touches Python, so there is nothing to
    // hand it to; holding it turns every `with_gil` on the trigger path
    // into a re-entrant check instead of an acquire. Never released —
    // the process exits with the simulation.
    unsafe {
        let _ = pyo3::ffi::PyGILState_Ensure();
    }
}

fn install_backend() {
    #[cfg(feature = "vpi")]
    rivet_vpi::startup();
    #[cfg(feature = "vhpi")]
    rivet_vhpi::startup();

    // The backend installs the Rust entry point; replace it with the one
    // that imports the Python testbench. `startup` returns without a
    // runtime when the simulator loads the library at compile time (VCS),
    // and there is nothing to install then.
    if rivet_core::runtime::is_initialised() {
        rivet_bridge::entry::install();
    }
}

unsafe extern "C" fn rivet_python_startup() {
    let r = std::panic::catch_unwind(|| {
        start_interpreter();
        install_backend();
    });
    if r.is_err() {
        eprintln!("rivet: the Python plugin failed to start");
    }
}

#[cfg(feature = "vpi")]
#[no_mangle]
pub static vlog_startup_routines: [Option<unsafe extern "C" fn()>; 2] = [Some(rivet_python_startup), None];

/// For simulators that need an explicit bootstrap symbol (Xcelium, CVC).
///
/// # Safety
/// Must be called from the simulator's thread, once, before anything else.
#[cfg(feature = "vpi")]
#[no_mangle]
pub unsafe extern "C" fn vlog_startup_routines_bootstrap() {
    rivet_python_startup();
}

#[cfg(feature = "vhpi")]
#[no_mangle]
pub static vhpi_startup_routines: [Option<unsafe extern "C" fn()>; 2] = [Some(rivet_python_startup), None];

/// For tools that need an explicit `-foreign` entry point (Questa, Riviera).
///
/// # Safety
/// Must be called from the simulator's thread, once, before anything else.
#[cfg(feature = "vhpi")]
#[no_mangle]
pub unsafe extern "C" fn vhpi_startup_routines_bootstrap() {
    rivet_python_startup();
}
