//! `import rivet` outside a simulator: the API, the kit and the mock
//! simulator, with no PLI attached.
//!
//! Inside a simulator this module is not used — the PLI plugin puts an
//! identical `_rivet` into the interpreter's inittab before anything
//! imports it, so a testbench reads the same either way.

use pyo3::prelude::*;

#[pymodule]
#[pyo3(name = "_rivet")]
fn rivet_ext(m: &Bound<'_, PyModule>) -> PyResult<()> {
    rivet_bridge::register_module(m)
}
