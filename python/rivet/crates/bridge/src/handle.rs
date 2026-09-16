//! `dut` and the things hanging off it.
//!
//! A handle is resolved once, against the real hierarchy, and then holds a
//! backend handle — so repeated reads cost a backend call and nothing
//! else. Each signal keeps a scratch vector and reads into it, so a read
//! in a per-cycle loop does not allocate.

use pyo3::exceptions::{PyAttributeError, PyIndexError, PyKeyError, PyRuntimeError};
use pyo3::prelude::*;
use rivet_core::backend::Action;
use rivet_core::handle::{Module, Object, Signal};
use rivet_core::value::LogicVec;
use std::cell::RefCell;

use crate::runtime_guard::{ensure_running, guard};
use crate::trigger::{PyTrigger, TriggerKind};
use crate::value;

fn not_found(e: rivet_core::Error) -> PyErr {
    PyAttributeError::new_err(e.to_string())
}

/// A hierarchical scope: the top level, a submodule, a generate block.
#[pyclass(name = "Module", module = "rivet", unsendable)]
#[derive(Clone)]
pub struct PyDut {
    pub inner: Module,
}

#[pymethods]
impl PyDut {
    /// A child signal by name. Raises `AttributeError` naming the scope if
    /// there is no such port — the misspelling is caught the first time
    /// the line runs, not at three in the morning.
    fn signal(&self, name: &str) -> PyResult<PySignal> {
        ensure_running()?;
        guard(|| self.inner.signal(name))?.map(PySignal::new).map_err(not_found)
    }

    /// A child scope by name.
    fn module(&self, name: &str) -> PyResult<PyDut> {
        ensure_running()?;
        guard(|| self.inner.module(name))?.map(|m| PyDut { inner: m }).map_err(not_found)
    }

    /// A signal at a dotted path: `dut.at("u_core.alu.result")`, with
    /// `[3]` indices allowed on any segment.
    fn at(&self, path: &str) -> PyResult<PySignal> {
        ensure_running()?;
        guard(|| self.inner.path_signal(path))?.map(PySignal::new).map_err(not_found)
    }

    /// A scope at a dotted path.
    fn scope_at(&self, path: &str) -> PyResult<PyDut> {
        ensure_running()?;
        guard(|| self.inner.path_module(path))?.map(|m| PyDut { inner: m }).map_err(not_found)
    }

    /// Element of an array of instances or a generate array.
    fn index(&self, i: i64) -> PyResult<PyDut> {
        ensure_running()?;
        let obj = guard(|| self.inner.index(i))?.map_err(|e| PyIndexError::new_err(e.to_string()))?;
        guard(|| obj.as_module())?.map(|m| PyDut { inner: m }).map_err(not_found)
    }

    fn has(&self, name: &str) -> PyResult<bool> {
        ensure_running()?;
        guard(|| self.inner.has_child(name))
    }

    /// The names of every immediate child, for discovery at the prompt.
    fn children(&self) -> PyResult<Vec<String>> {
        ensure_running()?;
        let kids = guard(|| self.inner.children())?.map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(kids.iter().map(Object::name).collect())
    }

    #[getter]
    fn name(&self) -> PyResult<String> {
        ensure_running()?;
        guard(|| self.inner.name())
    }

    #[getter]
    fn path(&self) -> PyResult<String> {
        ensure_running()?;
        guard(|| self.inner.path())
    }

    /// `dut["clk"]` for a name that is not a valid Python identifier, and
    /// for a name built at run time.
    fn __getitem__(&self, name: &str) -> PyResult<PyObject> {
        ensure_running()?;
        let obj = guard(|| self.inner.child(name))?.map_err(|e| PyKeyError::new_err(e.to_string()))?;
        Python::with_gil(|py| {
            if guard(|| obj.as_module())?.is_ok() {
                let m = guard(|| obj.as_module())?.map_err(not_found)?;
                Ok(PyDut { inner: m }.into_pyobject(py)?.into_any().unbind())
            } else {
                let s = guard(|| obj.as_signal())?.map_err(not_found)?;
                Ok(PySignal::new(s).into_pyobject(py)?.into_any().unbind())
            }
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!("<Module {}>", self.path()?))
    }
}

/// A value-carrying object: a net, a variable, a parameter.
#[pyclass(name = "Signal", module = "rivet", unsendable)]
pub struct PySignal {
    pub inner: Signal,
    /// Read into, not allocated per read.
    scratch: RefCell<LogicVec>,
    width: u32,
}

impl PySignal {
    pub fn new(inner: Signal) -> PySignal {
        let width = inner.width();
        PySignal { inner, scratch: RefCell::new(LogicVec::zeros(width)), width }
    }

    fn read(&self) -> PyResult<std::cell::Ref<'_, LogicVec>> {
        ensure_running()?;
        {
            let mut buf = self.scratch.borrow_mut();
            guard(|| self.inner.read_into(&mut buf))?;
        }
        Ok(self.scratch.borrow())
    }
}

#[pymethods]
impl PySignal {
    /// The value as an `int`. Raises `ValueError` if any bit is X or Z,
    /// rather than quietly reading them as zero.
    fn get<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let v = self.read()?;
        value::to_py_int(py, &v, &self.inner.path())
    }

    /// The value as an `int`, with X and Z read as 0.
    fn get_lossy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let v = self.read()?;
        Ok(value::to_py_int_lossy(py, &v))
    }

    /// The value as a two's-complement signed `int`.
    fn get_signed<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let v = self.read()?;
        value::to_py_int_signed(py, &v, &self.inner.path())
    }

    /// The value as a binary string, X and Z included: `"10x1"`.
    fn get_binstr(&self) -> PyResult<String> {
        Ok(self.read()?.to_binstr())
    }

    fn get_hexstr(&self) -> PyResult<String> {
        Ok(self.read()?.to_hexstr())
    }

    /// The four-state value, for bit-level work.
    fn get_vec(&self) -> PyResult<PyLogicVec> {
        Ok(PyLogicVec { inner: self.read()?.clone() })
    }

    fn is_resolvable(&self) -> PyResult<bool> {
        Ok(self.read()?.is_resolvable())
    }

    /// Inertial deposit, applied when the simulator next evaluates —
    /// reading back in the same phase returns the old value, exactly as in
    /// the Rust API and in cocotb.
    fn set(&self, v: &Bound<'_, PyAny>) -> PyResult<()> {
        ensure_running()?;
        let lv = value::to_logic_vec(v, self.width)?;
        guard(|| self.inner.set(lv))?;
        Ok(())
    }

    /// Immediate deposit (`vpiNoDelay`): visible to the next read.
    fn set_now(&self, v: &Bound<'_, PyAny>) -> PyResult<()> {
        ensure_running()?;
        let lv = value::to_logic_vec(v, self.width)?;
        guard(|| self.inner.set_now(lv))?;
        Ok(())
    }

    /// Override every driver until `release()`.
    fn force(&self, v: &Bound<'_, PyAny>) -> PyResult<()> {
        ensure_running()?;
        let lv = value::to_logic_vec(v, self.width)?;
        guard(|| self.inner.force(lv))?;
        Ok(())
    }

    fn release(&self) -> PyResult<()> {
        ensure_running()?;
        guard(|| self.inner.release())?;
        Ok(())
    }

    fn set_real(&self, v: f64) -> PyResult<()> {
        ensure_running()?;
        guard(|| self.inner.set_real(v))?;
        Ok(())
    }

    fn get_real(&self) -> PyResult<f64> {
        ensure_running()?;
        guard(|| self.inner.get_real())
    }

    fn get_string(&self) -> PyResult<String> {
        ensure_running()?;
        guard(|| self.inner.get_string())
    }

    /// The name of the current value's enumeration literal, when the
    /// simulator reports one (VHDL does; Verilog does not).
    fn enum_name(&self) -> PyResult<Option<String>> {
        ensure_running()?;
        guard(|| self.inner.enum_name())
    }

    /// Bits `hi` down to `lo`, inclusive. Reads and writes go through the
    /// whole vector, so it works on every backend.
    fn slice(&self, hi: u32, lo: u32) -> PyResult<PySlice> {
        ensure_running()?;
        if hi < lo || hi >= self.width.max(1) {
            return Err(PyIndexError::new_err(format!(
                "slice [{hi}:{lo}] is outside {} ({} bits)",
                self.inner.path(),
                self.width
            )));
        }
        Ok(PySlice { sig: self.inner, hi, lo })
    }

    /// Element of an unpacked array.
    fn index(&self, i: i64) -> PyResult<PySignal> {
        ensure_running()?;
        guard(|| self.inner.index(i))?.map(PySignal::new).map_err(|e| PyIndexError::new_err(e.to_string()))
    }

    /// Member of a struct-valued object.
    fn member(&self, name: &str) -> PyResult<PySignal> {
        ensure_running()?;
        guard(|| self.inner.member(name))?.map(PySignal::new).map_err(not_found)
    }

    /// Await the next transition to 1. `n` waits for that many, without
    /// returning to Python in between — the cheapest way to skip cycles.
    #[pyo3(signature = (n = 1))]
    fn rising_edge(&self, n: u64) -> PyTrigger {
        PyTrigger::new(TriggerKind::Edge {
            handle: self.inner.handle(),
            kind: rivet_core::runtime::EdgeKind::Rising,
            count: n.max(1),
        })
    }

    #[pyo3(signature = (n = 1))]
    fn falling_edge(&self, n: u64) -> PyTrigger {
        PyTrigger::new(TriggerKind::Edge {
            handle: self.inner.handle(),
            kind: rivet_core::runtime::EdgeKind::Falling,
            count: n.max(1),
        })
    }

    #[pyo3(signature = (n = 1))]
    fn value_change(&self, n: u64) -> PyTrigger {
        PyTrigger::new(TriggerKind::Edge {
            handle: self.inner.handle(),
            kind: rivet_core::runtime::EdgeKind::Any,
            count: n.max(1),
        })
    }

    #[getter]
    fn width(&self) -> u32 {
        self.width
    }

    #[getter]
    fn name(&self) -> PyResult<String> {
        ensure_running()?;
        guard(|| self.inner.name())
    }

    #[getter]
    fn path(&self) -> PyResult<String> {
        ensure_running()?;
        guard(|| self.inner.path())
    }

    #[getter]
    fn is_const(&self) -> PyResult<bool> {
        ensure_running()?;
        guard(|| self.inner.is_const())
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!("<Signal {} [{}]>", self.path()?, self.width))
    }

    fn __len__(&self) -> usize {
        self.width as usize
    }
}

/// A bit range of a signal.
#[pyclass(name = "Slice", module = "rivet", unsendable)]
pub struct PySlice {
    sig: Signal,
    hi: u32,
    lo: u32,
}

#[pymethods]
impl PySlice {
    fn get<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        ensure_running()?;
        let v = guard(|| self.sig.slice(self.hi, self.lo).get())?;
        value::to_py_int(py, &v, &format!("{}[{}:{}]", self.sig.path(), self.hi, self.lo))
    }

    /// Write these bits, leaving the rest of the signal alone. Successive
    /// slice writes in one time step compose.
    fn set(&self, v: &Bound<'_, PyAny>) -> PyResult<()> {
        ensure_running()?;
        let lv = value::to_logic_vec(v, self.hi - self.lo + 1)?;
        guard(|| self.sig.slice(self.hi, self.lo).set(lv))?;
        Ok(())
    }

    #[getter]
    fn width(&self) -> u32 {
        self.hi - self.lo + 1
    }

    fn __repr__(&self) -> PyResult<String> {
        ensure_running()?;
        Ok(format!("<Slice {}[{}:{}]>", self.sig.path(), self.hi, self.lo))
    }
}

/// A four-state vector, for the cases an `int` cannot express.
#[pyclass(name = "LogicVec", module = "rivet")]
#[derive(Clone)]
pub struct PyLogicVec {
    pub inner: LogicVec,
}

#[pymethods]
impl PyLogicVec {
    #[new]
    #[pyo3(signature = (value, width = None))]
    fn new(value: &Bound<'_, PyAny>, width: Option<u32>) -> PyResult<PyLogicVec> {
        // Without a width, a string keeps its own and a number gets the
        // smallest that holds it.
        let w = match width {
            Some(w) => w,
            None => match value.extract::<u128>() {
                Ok(v) => (128 - v.leading_zeros()).max(1),
                Err(_) => value.str()?.to_cow()?.chars().filter(|c| *c != '_').count() as u32,
            },
        };
        Ok(PyLogicVec { inner: value::to_logic_vec(value, w)? })
    }

    #[getter]
    fn width(&self) -> u32 {
        self.inner.width()
    }

    fn binstr(&self) -> String {
        self.inner.to_binstr()
    }

    fn hexstr(&self) -> String {
        self.inner.to_hexstr()
    }

    fn is_resolvable(&self) -> bool {
        self.inner.is_resolvable()
    }

    fn has_x(&self) -> bool {
        self.inner.has_x()
    }

    fn has_z(&self) -> bool {
        self.inner.has_z()
    }

    fn to_int<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        value::to_py_int(py, &self.inner, "this vector")
    }

    fn __int__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.to_int(py)
    }

    fn __len__(&self) -> usize {
        self.inner.width() as usize
    }

    fn __str__(&self) -> String {
        self.inner.to_binstr()
    }

    fn __repr__(&self) -> String {
        format!("LogicVec({:?})", self.inner.to_binstr())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(o) = other.extract::<PyRef<'_, PyLogicVec>>() {
            return Ok(self.inner == o.inner);
        }
        // Comparing against an int compares numbers, and an unresolved
        // vector is never equal to one.
        match (self.inner.to_u128(), other.extract::<u128>()) {
            (Ok(a), Ok(b)) => Ok(a == b),
            _ => Ok(false),
        }
    }
}

/// Deposit with an explicit action, for the rare testbench that needs one.
pub fn action_from_str(s: &str) -> PyResult<Action> {
    Ok(match s {
        "deposit" => Action::Deposit,
        "now" | "no_delay" => Action::NoDelay,
        "force" => Action::Force,
        "release" => Action::Release,
        _ => return Err(PyRuntimeError::new_err(format!("unknown write action {s:?}"))),
    })
}
