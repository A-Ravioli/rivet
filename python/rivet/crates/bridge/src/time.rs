//! Durations written the way a testbench writes them.
//!
//! `"10ns"`, `("10", "ns")`, `(1.5, "us")`, `42` steps. Parsed once, when
//! the testbench is written, not on every trigger — a `Duration` is
//! precision-independent, so it costs nothing to keep one around and
//! convert it at the moment the simulator's precision is known.

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyString, PyTuple};
use rivet_core::time::{Duration, Unit};

pub fn unit_from_str(s: &str) -> Option<Unit> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "step" | "steps" => Unit::Step,
        "fs" => Unit::Fs,
        "ps" => Unit::Ps,
        "ns" => Unit::Ns,
        "us" | "µs" => Unit::Us,
        "ms" => Unit::Ms,
        "s" | "sec" | "second" | "seconds" => Unit::Sec,
        _ => return None,
    })
}

/// Parse `"10ns"`, `"1.5 us"`, `"40 steps"`.
pub fn parse_str(s: &str) -> PyResult<Duration> {
    let t = s.trim();
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '_' || c == '-' || c == '+' || c == 'e' || c == 'E'))
        .ok_or_else(|| PyValueError::new_err(format!("{s:?} has no time unit — write it as \"10ns\" or \"1.5us\"")))?;
    let (num, unit) = t.split_at(split);
    let value: f64 = num
        .trim()
        .replace('_', "")
        .parse()
        .map_err(|_| PyValueError::new_err(format!("{s:?} does not start with a number")))?;
    let unit = unit_from_str(unit).ok_or_else(|| {
        PyValueError::new_err(format!("{:?} is not a time unit — use fs, ps, ns, us, ms, s or steps", unit.trim()))
    })?;
    check(value, s)?;
    Ok(Duration::new(value, unit))
}

fn check(value: f64, what: &str) -> PyResult<()> {
    if !value.is_finite() || value < 0.0 {
        return Err(PyValueError::new_err(format!("{what:?} is not a non-negative duration")));
    }
    Ok(())
}

/// Parse a duration from the forms the Python API accepts.
///
/// `units` is the optional second argument of `Timer(10, "ns")`. A bare
/// number with no unit anywhere is refused rather than guessed at: a
/// silent choice between nanoseconds and simulator steps is the kind of
/// mistake that only shows up as a waveform that is wrong by a factor of
/// a thousand.
pub fn parse(obj: &Bound<'_, PyAny>, units: Option<&str>) -> PyResult<Duration> {
    if let Ok(s) = obj.downcast::<PyString>() {
        let s = s.to_cow()?;
        return match units {
            // "10ns" with an explicit unit argument: the string wins only
            // if it carries one, otherwise treat it as the number.
            Some(u) => match parse_str(&s) {
                Ok(d) => Ok(d),
                Err(_) => with_units(s.trim().parse::<f64>().map_err(|_| bad(&s))?, u, &s),
            },
            None => parse_str(&s),
        };
    }
    if let Ok(t) = obj.downcast::<PyTuple>() {
        if t.len() == 2 {
            let value: f64 = t.get_item(0)?.extract()?;
            let unit: String = t.get_item(1)?.extract()?;
            return with_units(value, &unit, &format!("({value}, {unit:?})"));
        }
        return Err(PyValueError::new_err("a duration tuple is (value, units), for example (10, \"ns\")"));
    }
    if let Ok(d) = obj.extract::<PyRef<'_, PyDuration>>() {
        return Ok(d.inner);
    }
    if let Ok(value) = obj.extract::<f64>() {
        return match units {
            Some(u) => with_units(value, u, &format!("{value}")),
            None => Err(PyTypeError::new_err(format!(
                "{value} has no time unit — write Timer(\"{}ns\") or Timer({value}, \"ns\")",
                trim_float(value)
            ))),
        };
    }
    Err(PyTypeError::new_err(format!("cannot read {} as a duration", obj.get_type().name()?)))
}

fn trim_float(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn bad(s: &str) -> PyErr {
    PyValueError::new_err(format!("{s:?} is not a duration"))
}

fn with_units(value: f64, unit: &str, what: &str) -> PyResult<Duration> {
    let u = unit_from_str(unit).ok_or_else(|| {
        PyValueError::new_err(format!("{unit:?} is not a time unit — use fs, ps, ns, us, ms, s or steps"))
    })?;
    check(value, what)?;
    Ok(Duration::new(value, u))
}

/// A parsed duration, so a testbench can build one once and reuse it.
#[pyclass(name = "Duration", module = "rivet", frozen)]
#[derive(Clone)]
pub struct PyDuration {
    pub inner: Duration,
}

#[pymethods]
impl PyDuration {
    #[new]
    #[pyo3(signature = (value, units = None))]
    fn new(value: &Bound<'_, PyAny>, units: Option<&str>) -> PyResult<PyDuration> {
        Ok(PyDuration { inner: parse(value, units)? })
    }

    /// How many simulator steps this is, at the running simulator's
    /// precision. Needs a live simulation.
    fn steps(&self) -> PyResult<u64> {
        let precision = crate::runtime_guard::precision()?;
        Ok(self.inner.to_steps(precision))
    }

    #[getter]
    fn value(&self) -> f64 {
        self.inner.value
    }

    #[getter]
    fn units(&self) -> &'static str {
        self.inner.unit.suffix()
    }

    fn __repr__(&self) -> String {
        format!("Duration({}, {:?})", trim_float(self.inner.value), self.inner.unit.suffix())
    }

    fn __str__(&self) -> String {
        format!("{}", self.inner)
    }
}
