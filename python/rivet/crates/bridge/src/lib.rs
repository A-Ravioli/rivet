//! Rivet's Python bridge: write the testbench in Python, run the
//! scheduler in Rust.
//!
//! The arrangement is deliberately the opposite of cocotb's. There, the
//! interpreter owns the scheduler and the simulator calls into Python on
//! every callback. Here Rivet's native executor owns the scheduler, the
//! triggers are simulator callbacks, and a Python coroutine is just one
//! more task on it — entered once per `await` and not at all for the
//! parts that do not need it (clocks, multi-cycle waits, the kit).
//!
//! What ends up in the module:
//!
//! - [`handle`] — `dut`, signals, slices, four-state vectors
//! - [`trigger`] — the awaitables, and the futures they arm
//! - [`task`] — the coroutine driver, `start_soon`, `Task`
//! - [`registry`] — `@rivet.test`, turned into `TestSpec`s
//! - [`api`] — clocks, time, logging, how a test ends
//! - [`kit`] — memory, seeded random, scoreboards, coverage
//! - [`mock`] — a simulator in process, so this is testable without one
//! - [`entry`] — start-of-simulation, for the PLI plugin

pub mod api;
pub mod entry;
pub mod handle;
pub mod kit;
#[cfg(feature = "mock")]
pub mod mock;
pub mod registry;
pub mod runtime_guard;
pub mod task;
pub mod time;
pub mod trigger;
pub mod value;

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Record a test. Called by `rivet.test`, which is a thin wrapper in
/// Python so the decorator can carry a docstring and a signature.
#[pyfunction]
#[pyo3(signature = (
    func, *, name = None, timeout = None, timeout_units = None, skip = false,
    expect_fail = false, expect_fail_msg = None, expect_timeout = false,
    stage = 0, wall_timeout = None, param_sets = None,
))]
#[allow(clippy::too_many_arguments)]
fn register_test(
    func: &Bound<'_, PyAny>,
    name: Option<String>,
    timeout: Option<&Bound<'_, PyAny>>,
    timeout_units: Option<&str>,
    skip: bool,
    expect_fail: bool,
    expect_fail_msg: Option<String>,
    expect_timeout: bool,
    stage: i32,
    wall_timeout: Option<f64>,
    param_sets: Option<Vec<String>>,
) -> PyResult<()> {
    let (file, line) = registry::source_of(func);
    let timeout = timeout.map(|t| time::parse(t, timeout_units)).transpose()?;
    registry::push(registry::Entry {
        name: match name {
            Some(n) => n,
            None => registry::name_of(func)?,
        },
        module: registry::module_of(func),
        func: func.clone().unbind(),
        timeout,
        skip,
        expect_fail,
        expect_fail_msg,
        expect_timeout,
        stage,
        wall_timeout,
        param_sets: param_sets.unwrap_or_default(),
        file,
        line,
    });
    Ok(())
}

/// How many tests are registered.
#[pyfunction]
fn registered_count() -> usize {
    registry::count()
}

/// Forget every registered test. For a process that runs more than one
/// regression — the test suite of this bridge, mainly.
#[pyfunction]
fn clear_tests() {
    registry::clear();
}

/// The names of the registered tests, in the order they would run.
#[pyfunction]
fn registered_names() -> Vec<String> {
    registry::specs().iter().map(|s| s.full_name()).collect()
}

/// Whether a simulation is running right now.
#[pyfunction]
fn is_running() -> bool {
    rivet_core::runtime::is_initialised()
}

/// Populate a Python module with the whole surface. Shared by the
/// extension module and by the PLI plugin, so both expose the same names.
pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    rivet_core::log::init();
    runtime_guard::install_panic_hook();

    m.add_class::<handle::PyDut>()?;
    m.add_class::<handle::PySignal>()?;
    m.add_class::<handle::PySlice>()?;
    m.add_class::<handle::PyLogicVec>()?;
    m.add_class::<trigger::PyTrigger>()?;
    m.add_class::<trigger::TriggerIter>()?;
    m.add_class::<task::PyTask>()?;
    m.add_class::<api::PyClock>()?;
    m.add_class::<time::PyDuration>()?;

    m.add("SkipTest", m.py().get_type::<task::SkipTest>())?;
    m.add("TestFailure", m.py().get_type::<task::TestFailure>())?;
    m.add("TestSuccess", m.py().get_type::<task::TestSuccess>())?;
    m.add("CancelledError", m.py().get_type::<task::CancelledError>())?;

    m.add_function(wrap_pyfunction!(register_test, m)?)?;
    m.add_function(wrap_pyfunction!(registered_count, m)?)?;
    m.add_function(wrap_pyfunction!(registered_names, m)?)?;
    m.add_function(wrap_pyfunction!(clear_tests, m)?)?;
    m.add_function(wrap_pyfunction!(is_running, m)?)?;

    m.add_function(wrap_pyfunction!(api::timer, m)?)?;
    m.add_function(wrap_pyfunction!(api::read_write, m)?)?;
    m.add_function(wrap_pyfunction!(api::read_only, m)?)?;
    m.add_function(wrap_pyfunction!(api::next_time_step, m)?)?;
    m.add_function(wrap_pyfunction!(api::yield_now, m)?)?;
    m.add_function(wrap_pyfunction!(api::clock_cycles, m)?)?;
    m.add_function(wrap_pyfunction!(api::first, m)?)?;
    m.add_function(wrap_pyfunction!(api::start_soon, m)?)?;
    m.add_function(wrap_pyfunction!(api::now, m)?)?;
    m.add_function(wrap_pyfunction!(api::now_str, m)?)?;
    m.add_function(wrap_pyfunction!(api::precision, m)?)?;
    m.add_function(wrap_pyfunction!(api::simulator, m)?)?;
    m.add_function(wrap_pyfunction!(api::fail, m)?)?;
    m.add_function(wrap_pyfunction!(api::skip, m)?)?;
    m.add_function(wrap_pyfunction!(api::finish, m)?)?;
    m.add_function(wrap_pyfunction!(api::finish_now, m)?)?;
    m.add_function(wrap_pyfunction!(api::report_failure, m)?)?;
    m.add_function(wrap_pyfunction!(api::dump_tasks, m)?)?;
    m.add_function(wrap_pyfunction!(api::log_message, m)?)?;
    m.add_function(wrap_pyfunction!(api::test_seed, m)?)?;
    m.add_function(wrap_pyfunction!(api::base_seed, m)?)?;

    kit::register(m)?;
    #[cfg(feature = "mock")]
    mock::register(m)?;

    let version: &str = env!("CARGO_PKG_VERSION");
    m.add("__version__", version)?;
    let meta = PyDict::new(m.py());
    meta.set_item("mock", cfg!(feature = "mock"))?;
    m.add("build_info", meta)?;
    Ok(())
}
