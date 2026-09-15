//! A design and a simulator, in process, for testing the Python layer
//! itself and for trying an idea without a simulator installed.
//!
//! This is Rivet's own mock kernel — the one the Rust suites run against
//! — driven from Python. It schedules, evaluates and delivers callbacks
//! the way a real simulator does, so everything below the PLI boundary is
//! the same code that runs on Icarus.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rivet_core::backend::Handle;
use rivet_core::handle::Module;
use rivet_core::runtime;
use rivet_core::test::{run_regression_specs, Outcome, TestResult};
use rivet_mock::Design;
use std::cell::RefCell;
use std::rc::Rc;

use crate::registry;

/// A handle to a signal in a mock design.
#[pyclass(name = "MockSignal", module = "rivet.mock", frozen)]
#[derive(Clone, Copy)]
pub struct MockSignal {
    handle: Handle,
}

/// A design under construction.
#[pyclass(name = "MockDesign", module = "rivet.mock", unsendable)]
pub struct MockDesign {
    inner: RefCell<Option<Design>>,
}

fn borrow_design<R>(d: &MockDesign, f: impl FnOnce(&mut Design) -> R) -> PyResult<R> {
    let mut slot = d.inner.borrow_mut();
    let design = slot.as_mut().ok_or_else(|| PyRuntimeError::new_err("this design has already been run"))?;
    Ok(f(design))
}

#[pymethods]
impl MockDesign {
    #[new]
    #[pyo3(signature = (top = "top", precision = -12))]
    fn new(top: &str, precision: i32) -> MockDesign {
        MockDesign { inner: RefCell::new(Some(Design::new(top).precision(precision))) }
    }

    /// Add a signal of `width` bits to the top level.
    #[pyo3(signature = (name, width = 1))]
    fn logic(&self, name: &str, width: u32) -> PyResult<MockSignal> {
        borrow_design(self, |d| MockSignal { handle: d.logic(name, width) })
    }

    /// Set a signal's value before time zero.
    fn init(&self, sig: MockSignal, value: u64) -> PyResult<()> {
        borrow_design(self, |d| d.init(sig.handle, rivet_core::value::LogicVec::from_u64(64, value)))
    }

    /// Drive `sig` as a free-running clock from the kernel — a clock the
    /// design has, rather than one the testbench drives.
    #[pyo3(signature = (sig, half_period, start = 0))]
    fn clock(&self, sig: MockSignal, half_period: u64, start: u64) -> PyResult<()> {
        borrow_design(self, |d| d.clock(sig.handle, half_period, start))
    }

    /// A flip-flop: `q <= d` on each rising edge of `clk`.
    fn dff(&self, clk: MockSignal, d_in: MockSignal, q: MockSignal) -> PyResult<()> {
        borrow_design(self, |des| {
            let (c, di, qo) = (clk.handle, d_in.handle, q.handle);
            des.process(&[c], move |ctx| {
                if ctx.rose(c) {
                    let v = ctx.get(di);
                    ctx.nba(qo, v);
                }
            });
        })
    }

    /// A counter: synchronous active-low reset, count enable.
    fn counter(&self, clk: MockSignal, rst_n: MockSignal, en: MockSignal, count: MockSignal) -> PyResult<()> {
        borrow_design(self, |des| {
            let (c, r, e, q) = (clk.handle, rst_n.handle, en.handle, count.handle);
            des.process(&[c], move |ctx| {
                if !ctx.rose(c) {
                    return;
                }
                let width = 64;
                if ctx.get_u64(r) == 0 {
                    ctx.nba(q, rivet_core::value::LogicVec::from_u64(width, 0));
                } else if ctx.get_u64(e) != 0 {
                    let next = ctx.get_u64(q).wrapping_add(1);
                    ctx.nba(q, rivet_core::value::LogicVec::from_u64(width, next));
                }
            });
        })
    }

    /// Combinational `out = a + b`.
    fn adder(&self, a: MockSignal, b: MockSignal, out: MockSignal) -> PyResult<()> {
        borrow_design(self, |des| {
            let (ha, hb, ho) = (a.handle, b.handle, out.handle);
            des.process(&[ha, hb], move |ctx| {
                let v = ctx.get_u64(ha).wrapping_add(ctx.get_u64(hb));
                ctx.set(ho, rivet_core::value::LogicVec::from_u64(64, v));
            });
        })
    }

    /// Run every registered `@rivet.test` against this design and return
    /// one result dict per test. The design is consumed.
    #[pyo3(signature = (filter = None))]
    fn run(&self, py: Python<'_>, filter: Option<String>) -> PyResult<Vec<Py<PyDict>>> {
        let design = self
            .inner
            .borrow_mut()
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("this design has already been run"))?;
        let specs = registry::specs();
        if specs.is_empty() {
            return Err(PyValueError::new_err(
                "no tests are registered — decorate them with @rivet.test before calling run()",
            ));
        }
        let results = run_specs_on(design, specs, filter);
        results.iter().map(|r| result_dict(py, r)).collect()
    }
}

/// Drive the mock kernel through a Python regression.
fn run_specs_on(design: Design, specs: Vec<rivet_core::test::TestSpec>, filter: Option<String>) -> Vec<TestResult> {
    rivet_core::log::init();
    let mut sim = design.into_backend().install();
    let out: Rc<RefCell<Vec<TestResult>>> = Rc::new(RefCell::new(Vec::new()));
    let slot = out.clone();
    runtime::set_entry(move || {
        let root = runtime::backend(|b| b.root(None)).expect("mock design has a root");
        runtime::set_root(root);
        rivet_core::task::spawn_named("regression", async move {
            let r = run_regression_specs(Module::from_handle(root), specs, filter.as_deref()).await;
            *slot.borrow_mut() = r;
            runtime::finish();
        });
    });
    sim.run();
    runtime::shutdown();
    let v = out.borrow().clone();
    v
}

fn result_dict<'py>(py: Python<'py>, r: &TestResult) -> PyResult<Py<PyDict>> {
    let d = PyDict::new(py);
    d.set_item("name", &r.name)?;
    d.set_item("module", &r.module)?;
    let (status, message) = match &r.outcome {
        Outcome::Passed => ("passed", String::new()),
        Outcome::Skipped => ("skipped", String::new()),
        Outcome::Failed(m) => ("failed", m.clone()),
        // `Outcome` is `#[non_exhaustive]`: a new kind reads as a failure
        // rather than being silently dropped from the report.
        other => ("failed", format!("{other:?}")),
    };
    d.set_item("status", status)?;
    d.set_item("message", message)?;
    d.set_item("sim_time_steps", r.sim_time_steps)?;
    d.set_item("wall_secs", r.wall_secs)?;
    d.set_item("seed", r.seed)?;
    d.set_item("file", &r.file)?;
    d.set_item("line", r.line)?;
    Ok(d.unbind())
}

pub fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "mock")?;
    m.add_class::<MockDesign>()?;
    m.add_class::<MockSignal>()?;
    parent.add_submodule(&m)?;
    // The `rivet` package republishes this as `rivet.mock`; a submodule
    // built at run time is not importable on its own.
    Ok(())
}
