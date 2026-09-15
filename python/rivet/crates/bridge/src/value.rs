//! Python values to and from [`LogicVec`].
//!
//! cocotb moves every value through a decimal or binary *string*, which is
//! most of why its read/write row costs 305 µs a cycle. Here a value that
//! fits in a machine word never leaves the integer domain, and a wider one
//! moves as raw little-endian bytes through `int.to_bytes` — one
//! allocation, no digits.

use pyo3::exceptions::{PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyInt, PyString};
use rivet_core::value::{Logic, LogicVec};

/// Convert a Python object to a `width`-bit vector.
///
/// Accepts `int` (any size, negative taken as two's complement), `bool`,
/// and `str` — `"1010"`, `"0b1010"`, `"0xdead"`, `"x"`, `"10xz"` — so a
/// testbench can write X and Z as well as numbers.
pub fn to_logic_vec(obj: &Bound<'_, PyAny>, width: u32) -> PyResult<LogicVec> {
    // `bool` is a subclass of `int`, and the common case besides.
    if let Ok(b) = obj.downcast::<PyBool>() {
        return Ok(LogicVec::from_u64(width, b.is_true() as u64));
    }
    if let Ok(i) = obj.downcast::<PyInt>() {
        return int_to_logic_vec(i, width);
    }
    if let Ok(s) = obj.downcast::<PyString>() {
        let s = s.to_cow()?;
        return parse_string(&s, width);
    }
    if let Ok(v) = obj.extract::<PyRef<'_, crate::handle::PyLogicVec>>() {
        return Ok(resize(v.inner.clone(), width));
    }
    Err(PyTypeError::new_err(format!(
        "cannot write {} to a {width}-bit signal: expected int, bool, str or LogicVec",
        obj.get_type().name()?
    )))
}

/// `1 << n` as a Python int, for two's-complement arithmetic on values
/// too wide for a machine word.
fn one_shifted<'py>(py: Python<'py>, n: u32) -> PyResult<Bound<'py, PyAny>> {
    1u8.into_pyobject(py)?.into_any().call_method1("__lshift__", (n,))
}

/// Parse the string forms a testbench writes.
///
/// Verilog literals (`"8'hA5"`, `"4'b10xz"`), bare four-state digits
/// (`"1010xxzz"`), and — because this is Python — the prefixes a Python
/// programmer reaches for first: `"0xA5"`, `"0b1010"`, `"0o777"`.
///
/// A bare string shorter than the signal is zero-extended, as in the Rust
/// API: `"x"` on an eight-bit signal is one X bit above seven zeros, not
/// eight X bits. Write `"xxxxxxxx"` or `"8'hxx"` for that.
fn parse_string(s: &str, width: u32) -> PyResult<LogicVec> {
    let t = s.trim();
    let rest = |n: usize| t[n..].replace('_', "");
    let parsed = if t.len() > 2 && (t.starts_with("0x") || t.starts_with("0X")) {
        LogicVec::parse(&format!("{width}'h{}", rest(2)))
    } else if t.len() > 2 && (t.starts_with("0b") || t.starts_with("0B")) {
        LogicVec::parse(&format!("{width}'b{}", rest(2)))
    } else if t.len() > 2 && (t.starts_with("0o") || t.starts_with("0O")) {
        u128::from_str_radix(&rest(2), 8).ok().map(|n| LogicVec::from_u128(width, n))
    } else {
        LogicVec::parse(t)
    };
    parsed.map(|v| resize(v, width)).ok_or_else(|| {
        PyValueError::new_err(format!(
            "cannot parse {s:?} as a {width}-bit value — write digits (\"1010xxzz\"), a Verilog \
             literal (\"8'hA5\") or a Python one (\"0xA5\")"
        ))
    })
}

fn resize(mut v: LogicVec, width: u32) -> LogicVec {
    if v.width() != width {
        v.resize(width);
    }
    v
}

fn int_to_logic_vec(i: &Bound<'_, PyInt>, width: u32) -> PyResult<LogicVec> {
    // Fast paths: almost every signal in a testbench is 64 bits or less.
    if width <= 64 {
        if let Ok(u) = i.extract::<u64>() {
            return Ok(LogicVec::from_u64(width, u));
        }
        if let Ok(s) = i.extract::<i64>() {
            return Ok(LogicVec::from_i64(width, s));
        }
    }
    if let Ok(u) = i.extract::<u128>() {
        return Ok(LogicVec::from_u128(width, u));
    }
    // Wide, or negative and wide: go through raw bytes rather than digits.
    // Two's complement of the signal's own width, so `-1` fills with ones.
    let negative = i.lt(0i64)?;
    let nbytes = width.div_ceil(8).max(1) as usize;
    let py = i.py();
    let value = if negative {
        // `(x + 2**width) % 2**width` without formatting anything.
        let modulus = one_shifted(py, width)?;
        i.call_method1("__mod__", (modulus,))?
    } else {
        i.clone().into_any()
    };
    let bytes = value.call_method1("to_bytes", (nbytes, "little"))?;
    let bytes: &Bound<'_, PyBytes> = bytes.downcast().map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(from_le_bytes(bytes.as_bytes(), width))
}

/// Build a vector from little-endian bytes, truncating to `width`.
pub fn from_le_bytes(bytes: &[u8], width: u32) -> LogicVec {
    let words = width.div_ceil(32).max(1) as usize;
    let mut aval = vec![0u32; words];
    for (i, b) in bytes.iter().enumerate() {
        let w = i / 4;
        if w >= words {
            break;
        }
        aval[w] |= (*b as u32) << ((i % 4) * 8);
    }
    LogicVec::from_planes(width, aval, vec![0u32; words])
}

/// Convert a resolved vector to a Python `int`. X or Z bits raise
/// `ValueError`, naming the value, the same way Rust's `get_u64` returns
/// `Unresolved` rather than guessing.
pub fn to_py_int<'py>(py: Python<'py>, v: &LogicVec, what: &str) -> PyResult<Bound<'py, PyAny>> {
    if !v.is_resolvable() {
        return Err(PyValueError::new_err(format!("{what} is {} — it has X or Z bits", v.to_binstr())));
    }
    Ok(unresolved_to_py_int(py, v))
}

/// As [`to_py_int`], but X and Z read as 0 (cocotb's `.integer` on a
/// partially-unknown value, and Rust's `get_u64_lossy`).
pub fn to_py_int_lossy<'py>(py: Python<'py>, v: &LogicVec) -> Bound<'py, PyAny> {
    unresolved_to_py_int(py, v)
}

fn unresolved_to_py_int<'py>(py: Python<'py>, v: &LogicVec) -> Bound<'py, PyAny> {
    if v.width() <= 64 {
        return v.to_u64_lossy().into_pyobject(py).unwrap().into_any();
    }
    // Wide: hand CPython the aval plane as bytes and let it build the int.
    let aval = v.aval();
    let mut bytes = Vec::with_capacity(aval.len() * 4);
    for w in aval {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    let b = PyBytes::new(py, &bytes);
    py.get_type::<PyInt>()
        .call_method1("from_bytes", (b, "little"))
        .expect("int.from_bytes on a little-endian byte string")
}

/// Signed reading of a vector, as a Python `int`.
pub fn to_py_int_signed<'py>(py: Python<'py>, v: &LogicVec, what: &str) -> PyResult<Bound<'py, PyAny>> {
    if !v.is_resolvable() {
        return Err(PyValueError::new_err(format!("{what} is {} — it has X or Z bits", v.to_binstr())));
    }
    if v.width() <= 64 {
        let s = v.to_i64().map_err(|e| PyValueError::new_err(e.to_string()))?;
        return Ok(s.into_pyobject(py).unwrap().into_any());
    }
    let unsigned = unresolved_to_py_int(py, v);
    // Subtract 2**width when the top bit is set.
    if v.bit(v.width() - 1) == Logic::One {
        let modulus = one_shifted(py, v.width())?;
        return unsigned.call_method1("__sub__", (modulus,));
    }
    Ok(unsigned)
}

/// `int` too large for the target, reported the way Python reports it.
pub fn overflow(what: &str, width: u32) -> PyErr {
    PyOverflowError::new_err(format!("value does not fit in {width} bits ({what})"))
}
