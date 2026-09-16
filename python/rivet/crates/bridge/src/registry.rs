//! `@rivet.test` — collecting Python tests and handing them to Rivet's
//! own regression loop.
//!
//! The decorator records a description; nothing runs at import time. When
//! the simulation starts, each description becomes a
//! [`rivet_core::test::TestSpec`], so Python tests are seeded, timed out,
//! traced and written to `results.xml` by exactly the same code that runs
//! Rust ones.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use rivet_core::handle::Module;
use rivet_core::test::{TestFuture, TestSpec};
use rivet_core::time::Duration;
use std::cell::RefCell;

use crate::handle::PyDut;
use crate::task::{format_exception, py_err_to_rivet, PyCoroDriver, TestSuccess};

pub struct Entry {
    pub func: Py<PyAny>,
    pub name: String,
    pub module: String,
    pub timeout: Option<Duration>,
    pub skip: bool,
    pub expect_fail: bool,
    pub expect_fail_msg: Option<String>,
    pub expect_timeout: bool,
    pub stage: i32,
    pub wall_timeout: Option<f64>,
    pub param_sets: Vec<String>,
    pub file: String,
    pub line: u32,
}

thread_local! {
    static TESTS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

pub fn push(e: Entry) {
    TESTS.with(|t| t.borrow_mut().push(e));
}

pub fn count() -> usize {
    TESTS.with(|t| t.borrow().len())
}

pub fn clear() {
    TESTS.with(|t| t.borrow_mut().clear());
}

/// The registered tests, in registration order, as specs the regression
/// loop can run. Stage order is applied by the loop's caller, matching
/// `all_tests()` for Rust.
pub fn specs() -> Vec<TestSpec> {
    TESTS.with(|t| {
        let mut entries: Vec<usize> = (0..t.borrow().len()).collect();
        // Stable sort by stage only: within a stage, registration order is
        // source order, which is what a reader expects.
        entries.sort_by_key(|&i| t.borrow()[i].stage);
        entries.iter().map(|&i| spec_for(&t.borrow()[i])).collect()
    })
}

fn spec_for(e: &Entry) -> TestSpec {
    let func = Python::with_gil(|py| e.func.clone_ref(py));
    let full = format!("{}::{}", e.module, e.name);
    let timeout = e.timeout;
    TestSpec {
        name: e.name.clone(),
        module: e.module.clone(),
        run: Box::new(move |dut: Module| -> TestFuture {
            let func = Python::with_gil(|py| func.clone_ref(py));
            let full = full.clone();
            Box::pin(async move {
                // Calling the test function produces the coroutine; it
                // does not run a line of it yet.
                let coro = Python::with_gil(|py| -> Result<Py<PyAny>, rivet_core::Error> {
                    let dut = PyDut { inner: dut }.into_pyobject(py).map_err(|e| py_err_to_rivet(py, e))?;
                    let obj = func.call1(py, (dut,)).map_err(|e| py_err_to_rivet(py, e))?;
                    if !obj.bind(py).hasattr(pyo3::intern!(py, "send")).unwrap_or(false) {
                        return Err(rivet_core::Error::Msg(format!(
                            "{full} is not a coroutine function — a Rivet test must be `async def`"
                        )));
                    }
                    Ok(obj)
                })?;
                match PyCoroDriver::new(coro).await {
                    Ok(_) => Ok(()),
                    Err(e) => Python::with_gil(|py| {
                        // `rivet.finish()` raised inside the test: the
                        // test is over and it passed.
                        if e.is_instance_of::<TestSuccess>(py) {
                            log::info!("{full}: {}", format_exception(py, &e));
                            Ok(())
                        } else {
                            Err(py_err_to_rivet(py, e))
                        }
                    }),
                }
            })
        }),
        timeout: Box::new(move || timeout),
        skip: e.skip,
        expect_fail: e.expect_fail,
        expect_fail_msg: e.expect_fail_msg.clone(),
        expect_timeout: e.expect_timeout,
        file: e.file.clone(),
        line: e.line,
        stage: e.stage,
        wall_timeout: e.wall_timeout,
        param_sets: e.param_sets.clone(),
    }
}

/// Where a function was written, for `results.xml` and for editors that
/// turn a failure into a clickable line.
pub fn source_of(func: &Bound<'_, PyAny>) -> (String, u32) {
    let code = match func.getattr(pyo3::intern!(func.py(), "__code__")) {
        Ok(c) => c,
        Err(_) => return (String::new(), 0),
    };
    let file = code.getattr("co_filename").and_then(|f| f.extract::<String>()).unwrap_or_default();
    let line = code.getattr("co_firstlineno").and_then(|l| l.extract::<u32>()).unwrap_or(0);
    (file, line)
}

pub fn module_of(func: &Bound<'_, PyAny>) -> String {
    func.getattr(pyo3::intern!(func.py(), "__module__"))
        .and_then(|m| m.extract::<String>())
        .unwrap_or_else(|_| "__main__".to_string())
}

pub fn name_of(func: &Bound<'_, PyAny>) -> PyResult<String> {
    func.getattr(pyo3::intern!(func.py(), "__name__"))
        .and_then(|m| m.extract::<String>())
        .map_err(|_| PyTypeError::new_err("@rivet.test must decorate a function"))
}
