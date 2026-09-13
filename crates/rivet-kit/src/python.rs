//! A Python function as a reference model.
//!
//! Teams moving from cocotb usually already have a model in Python, often
//! built on numpy. This lets a Rivet test score against it without
//! rewriting it: the interpreter is embedded, the call happens inside the
//! test's own task, and the GIL is taken only for the duration of the
//! call, so the simulator hot path is untouched.
//!
//! Enable the `python` feature. The crate then links libpython, so leave
//! it off unless a test needs it.
//!
//! ```ignore
//! let model = PyModel::from_source("def step(x): return (x * 3) & 0xff", "step")?;
//! let mut sb = ModelScoreboard::new("alu", model);
//! sb.drive(7u64);
//! sb.observe(dut_result);
//! ```

use crate::model::Model;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyModule};
use rivet_core::error::{Error, Result};
use std::ffi::CString;

/// A Python callable used as a model.
pub struct PyModel {
    step: Py<PyAny>,
    reset: Option<Py<PyAny>>,
    name: String,
}

fn err(e: PyErr) -> Error {
    Error::Msg(format!("python model: {e}"))
}

impl PyModel {
    /// Load a function from Python source given inline.
    pub fn from_source(source: &str, function: &str) -> Result<PyModel> {
        Python::with_gil(|py| {
            let code = CString::new(source).map_err(|_| Error::Msg("NUL in python source".into()))?;
            let file = CString::new("rivet_model.py").unwrap();
            let name = CString::new("rivet_model").unwrap();
            let module = PyModule::from_code(py, &code, &file, &name).map_err(err)?;
            Self::from_module(py, &module, function)
        })
    }

    /// Load a function from a Python file. The file's directory is added to
    /// `sys.path`, so the model can import its own helpers.
    pub fn from_file(path: impl AsRef<std::path::Path>, function: &str) -> Result<PyModel> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path).map_err(|e| Error::Msg(format!("{}: {e}", path.display())))?;
        Python::with_gil(|py| {
            if let Some(dir) = path.parent() {
                let sys = py.import("sys").map_err(err)?;
                let sys_path = sys.getattr("path").map_err(err)?;
                let list: &Bound<PyList> = sys_path.downcast().map_err(|e| Error::Msg(e.to_string()))?;
                list.insert(0, dir.display().to_string()).map_err(err)?;
            }
            let code = CString::new(source).map_err(|_| Error::Msg("NUL in python source".into()))?;
            let file = CString::new(path.display().to_string()).unwrap();
            let name = CString::new("rivet_model").unwrap();
            let module = PyModule::from_code(py, &code, &file, &name).map_err(err)?;
            Self::from_module(py, &module, function)
        })
    }

    fn from_module(py: Python<'_>, module: &Bound<'_, PyModule>, function: &str) -> Result<PyModel> {
        let step = module.getattr(function).map_err(err)?;
        if !step.is_callable() {
            return Err(Error::Msg(format!("python model: {function} is not callable")));
        }
        // An optional `reset()` is called when the scoreboard resets.
        let reset = module.getattr("reset").ok().filter(|r| r.is_callable()).map(|r| r.unbind());
        let _ = py;
        Ok(PyModel { step: step.unbind(), reset, name: function.to_string() })
    }

    /// The function's name, for error messages.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Call the model with one unsigned argument.
    pub fn call_u64(&self, input: u64) -> Result<Option<u64>> {
        Python::with_gil(|py| {
            let out = self.step.call1(py, (input,)).map_err(err)?;
            if out.is_none(py) {
                return Ok(None);
            }
            out.extract::<u64>(py).map(Some).map_err(err)
        })
    }

    /// Call the model with a list of unsigned arguments.
    pub fn call_slice(&self, input: &[u64]) -> Result<Option<Vec<u64>>> {
        Python::with_gil(|py| {
            let out = self.step.call1(py, (input.to_vec(),)).map_err(err)?;
            if out.is_none(py) {
                return Ok(None);
            }
            out.extract::<Vec<u64>>(py).map(Some).map_err(err)
        })
    }
}

impl Model<u64, u64> for PyModel {
    fn step(&mut self, input: &u64) -> Option<u64> {
        self.call_u64(*input).unwrap_or_else(|e| panic!("{e}"))
    }

    fn reset(&mut self) {
        if let Some(r) = &self.reset {
            Python::with_gil(|py| {
                let _ = r.call0(py).map_err(|e| panic!("python model reset: {e}"));
            });
        }
    }
}

impl Model<Vec<u64>, Vec<u64>> for PyModel {
    fn step(&mut self, input: &Vec<u64>) -> Option<Vec<u64>> {
        self.call_slice(input).unwrap_or_else(|e| panic!("{e}"))
    }

    fn reset(&mut self) {
        if let Some(r) = &self.reset {
            Python::with_gil(|py| {
                let _ = r.call0(py).map_err(|e| panic!("python model reset: {e}"));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelScoreboard;

    #[test]
    fn scalar_model_scores_a_stream() {
        let model = PyModel::from_source("def triple(x):\n    return (x * 3) & 0xff\n", "triple").unwrap();
        let mut sb: ModelScoreboard<u64, u64, PyModel> = ModelScoreboard::new("triple", model);
        for i in 0..8u64 {
            sb.drive(i);
            sb.observe((i * 3) & 0xff);
        }
        sb.finish().unwrap();
    }

    #[test]
    fn a_wrong_design_is_caught() {
        let model = PyModel::from_source("def inc(x):\n    return x + 1\n", "inc").unwrap();
        let mut sb: ModelScoreboard<u64, u64, PyModel> = ModelScoreboard::new("inc", model);
        sb.drive(1);
        sb.observe(3);
        assert!(sb.finish().is_err(), "the model said 2, the design said 3");
    }

    #[test]
    fn a_model_may_return_none_and_may_keep_state() {
        let src = "
acc = 0

def push(x):
    global acc
    acc += x
    if acc < 10:
        return None
    out = acc
    acc = 0
    return out

def reset():
    global acc
    acc = 0
";
        let model = PyModel::from_source(src, "push").unwrap();
        assert!(model.reset.is_some(), "an optional reset() is picked up");
        let mut sb: ModelScoreboard<u64, u64, PyModel> = ModelScoreboard::new("push", model);
        // 4 + 4 = 8 produces nothing; the third push crosses ten.
        sb.drive(4);
        sb.drive(4);
        sb.drive(4);
        sb.observe(12);
        sb.finish().unwrap();
    }

    #[test]
    fn vector_models_round_trip() {
        let model = PyModel::from_source("def rev(xs):\n    return list(reversed(xs))\n", "rev").unwrap();
        assert_eq!(model.call_slice(&[1, 2, 3]).unwrap(), Some(vec![3, 2, 1]));
    }

    #[test]
    fn a_missing_function_is_an_error_not_a_panic() {
        assert!(PyModel::from_source("x = 1\n", "nope").is_err());
    }
}
