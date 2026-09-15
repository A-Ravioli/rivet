//! The simulator-independent kit: memory, seeded random, scoreboards and
//! functional coverage.
//!
//! These are the same Rust implementations the Rust testbenches use, so a
//! Python test and a Rust one score against the same scoreboard and merge
//! into the same coverage database. None of them touch the simulator, so
//! none of them cost a trigger.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList};
use rivet_core::random::Rng as CoreRng;
use rivet_kit::memory::Memory as CoreMemory;

fn err(e: rivet_core::Error) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// Sparse byte memory shared by handle (see `rivet_kit::Memory`).
#[pyclass(module = "rivet", name = "Memory", unsendable)]
#[derive(Clone)]
struct Memory {
    inner: CoreMemory,
}

#[pymethods]
impl Memory {
    #[new]
    #[pyo3(signature = (fill = 0))]
    fn new(fill: u8) -> Memory {
        Memory { inner: CoreMemory::with_fill(fill) }
    }

    fn read_u8(&self, addr: u64) -> u8 {
        self.inner.read_u8(addr)
    }
    fn write_u8(&self, addr: u64, v: u8) {
        self.inner.write_u8(addr, v)
    }
    /// Little-endian word of `bytes` bytes (1..=8).
    fn read_word(&self, addr: u64, bytes: u32) -> u64 {
        self.inner.read_word(addr, bytes)
    }
    /// Write a little-endian word; only bytes whose `strobe` bit is set.
    #[pyo3(signature = (addr, bytes, value, strobe = u64::MAX))]
    fn write_word(&self, addr: u64, bytes: u32, value: u64, strobe: u64) {
        self.inner.write_word(addr, bytes, value, strobe)
    }
    fn read_u32(&self, addr: u64) -> u32 {
        self.inner.read_u32(addr)
    }
    fn write_u32(&self, addr: u64, v: u32) {
        self.inner.write_u32(addr, v)
    }
    fn read_u64(&self, addr: u64) -> u64 {
        self.inner.read_u64(addr)
    }
    fn write_u64(&self, addr: u64, v: u64) {
        self.inner.write_u64(addr, v)
    }
    fn read_bytes<'py>(&self, py: Python<'py>, addr: u64, n: usize) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.read_vec(addr, n))
    }
    fn write_bytes(&self, addr: u64, data: &[u8]) {
        self.inner.write_bytes(addr, data)
    }
    /// Load a `$readmemh` file; returns the number of words written.
    #[pyo3(signature = (path, base = 0, word_bytes = 4))]
    fn load_hex(&self, path: &str, base: u64, word_bytes: u32) -> PyResult<usize> {
        self.inner.load_hex(path, base, word_bytes).map_err(err)
    }
    #[pyo3(signature = (path, base = 0))]
    fn load_bin(&self, path: &str, base: u64) -> PyResult<usize> {
        self.inner.load_bin(path, base).map_err(err)
    }
    fn dump_hex(&self, path: &str, base: u64, words: u64, word_bytes: u32) -> PyResult<()> {
        self.inner.dump_hex(path, base, words, word_bytes).map_err(err)
    }
    fn pages(&self) -> usize {
        self.inner.pages()
    }
    fn clear(&self) {
        self.inner.clear()
    }
}

/// Deterministic xoshiro256** generator, identical to Rivet's Rust one.
#[pyclass(module = "rivet", name = "Rng", unsendable)]
struct Rng {
    inner: CoreRng,
}

#[pymethods]
impl Rng {
    #[new]
    fn new(seed: u64) -> Rng {
        Rng { inner: CoreRng::seed_from_u64(seed) }
    }
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }
    /// Uniform integer in `[lo, hi]`.
    fn randint(&mut self, lo: i64, hi: i64) -> PyResult<i64> {
        if lo > hi {
            return Err(PyValueError::new_err(format!("empty range {lo}..={hi}")));
        }
        Ok(self.inner.gen_range(lo..=hi))
    }
    /// Uniform float in `[0, 1)`.
    fn random(&mut self) -> f64 {
        self.inner.gen::<f64>()
    }
    fn gen_bool(&mut self, p: f64) -> bool {
        self.inner.gen_bool(p)
    }
    fn choice(&mut self, py: Python<'_>, items: Vec<PyObject>) -> PyResult<PyObject> {
        self.inner.choose(&items).map(|o| o.clone_ref(py)).ok_or_else(|| PyValueError::new_err("empty sequence"))
    }
    fn shuffle(&mut self, items: Vec<PyObject>) -> Vec<PyObject> {
        let mut v = items;
        self.inner.shuffle(&mut v);
        v
    }
    fn bytes<'py>(&mut self, py: Python<'py>, n: usize) -> Bound<'py, PyBytes> {
        let mut v = vec![0u8; n];
        self.inner.fill_bytes(&mut v);
        PyBytes::new(py, &v)
    }
    /// An independent stream derived from this one.
    fn fork(&mut self) -> Rng {
        Rng { inner: self.inner.fork() }
    }
}

/// The seed Rivet would give a test named `full_name` under `base`.
#[pyfunction]
fn seed_for_test(base: u64, full_name: &str) -> u64 {
    rivet_core::random::seed_for_test(base, full_name)
}

/// A Python object compared with `==` inside the Rust scoreboard.
struct Item(PyObject);

impl Clone for Item {
    fn clone(&self) -> Item {
        Python::with_gil(|py| Item(self.0.clone_ref(py)))
    }
}

impl PartialEq for Item {
    fn eq(&self, other: &Item) -> bool {
        Python::with_gil(|py| self.0.bind(py).eq(other.0.bind(py)).unwrap_or(false))
    }
}

impl std::fmt::Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Python::with_gil(|py| match self.0.bind(py).repr() {
            Ok(r) => f.write_str(&r.to_string_lossy()),
            Err(_) => f.write_str("<object>"),
        })
    }
}

/// In-order scoreboard of Python objects.
#[pyclass(module = "rivet", name = "Scoreboard", unsendable)]
struct Scoreboard {
    inner: rivet_kit::Scoreboard<Item>,
}

#[pymethods]
impl Scoreboard {
    #[new]
    fn new(name: &str) -> Scoreboard {
        Scoreboard { inner: rivet_kit::Scoreboard::new(name) }
    }
    fn expect(&self, item: PyObject) {
        self.inner.expect(Item(item))
    }
    fn observe(&self, item: PyObject) {
        self.inner.observe(Item(item))
    }
    fn matched(&self) -> usize {
        self.inner.matched()
    }
    fn pending(&self) -> usize {
        self.inner.pending()
    }
    fn errors(&self) -> Vec<String> {
        self.inner.errors()
    }
    /// Raises `RuntimeError` on mismatches or unobserved expectations.
    fn finish(&self) -> PyResult<()> {
        self.inner.finish().map_err(err)
    }
}

/// Bin specification for a coverpoint (builder; every method returns self).
#[pyclass(module = "rivet", name = "Bins", unsendable)]
#[derive(Clone, Default)]
struct Bins {
    inner: rivet_core::coverage::Bins,
}

#[pymethods]
impl Bins {
    #[new]
    fn new() -> Bins {
        Bins::default()
    }
    fn bin(&self, name: &str, lo: i64, hi: i64) -> Bins {
        Bins { inner: self.inner.clone().bin(name, lo..=hi) }
    }
    fn values(&self, name: &str, values: Vec<i64>) -> Bins {
        Bins { inner: self.inner.clone().values(name, values) }
    }
    fn auto(&self, prefix: &str, lo: i64, hi: i64) -> Bins {
        Bins { inner: self.inner.clone().auto(prefix, lo..=hi) }
    }
    fn split(&self, prefix: &str, lo: i64, hi: i64, n: u32) -> Bins {
        Bins { inner: self.inner.clone().split(prefix, lo..=hi, n) }
    }
    fn ignore(&self, lo: i64, hi: i64) -> Bins {
        Bins { inner: self.inner.clone().ignore(lo..=hi) }
    }
    fn illegal(&self, lo: i64, hi: i64) -> Bins {
        Bins { inner: self.inner.clone().illegal(lo..=hi) }
    }
    fn __len__(&self) -> usize {
        self.inner.len()
    }
}

#[pyclass(module = "rivet", name = "CoverPoint", unsendable)]
struct CoverPoint {
    inner: rivet_core::coverage::CoverPoint,
}

#[pymethods]
impl CoverPoint {
    /// Record a value; returns False if it matched no bin.
    fn sample(&self, value: i64) -> bool {
        self.inner.sample(value)
    }
    fn hits(&self) -> Vec<(String, u64)> {
        self.inner.hits()
    }
    fn percent(&self) -> f64 {
        self.inner.percent()
    }
    fn name(&self) -> String {
        self.inner.name()
    }
}

#[pyclass(module = "rivet", name = "Cross", unsendable)]
struct Cross {
    inner: rivet_core::coverage::Cross,
}

#[pymethods]
impl Cross {
    fn hits(&self) -> Vec<(String, u64)> {
        self.inner.hits()
    }
    fn percent(&self) -> f64 {
        self.inner.percent()
    }
}

#[pyclass(module = "rivet", name = "Covergroup", unsendable)]
struct Covergroup {
    inner: rivet_core::coverage::Covergroup,
}

#[pymethods]
impl Covergroup {
    #[new]
    fn new(name: &str) -> Covergroup {
        Covergroup { inner: rivet_core::coverage::Covergroup::new(name) }
    }
    fn point(&self, name: &str, bins: &Bins) -> CoverPoint {
        CoverPoint { inner: self.inner.point(name, bins.inner.clone()) }
    }
    fn cross(&self, name: &str, points: Vec<PyRef<CoverPoint>>) -> Cross {
        let refs: Vec<&rivet_core::coverage::CoverPoint> = points.iter().map(|p| &p.inner).collect();
        Cross { inner: self.inner.cross(name, &refs) }
    }
    fn percent(&self) -> f64 {
        self.inner.percent()
    }
    fn counts(&self) -> (usize, usize) {
        self.inner.counts()
    }
}

/// Overall functional coverage of every covergroup, in percent.
#[pyfunction]
fn coverage_percent() -> f64 {
    rivet_core::coverage::percent()
}

/// The coverage registry as the JSON `rivet cov report` reads.
#[pyfunction]
fn coverage_json() -> String {
    rivet_core::coverage::to_json()
}

#[pyfunction]
fn coverage_table() -> String {
    rivet_core::coverage::render_table()
}

#[pyfunction]
fn write_coverage(path: &str) -> PyResult<()> {
    rivet_core::coverage::write_json(std::path::Path::new(path)).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction]
fn clear_coverage() {
    rivet_core::coverage::clear()
}

/// Parse `$readmemh` text into `(index, value)` pairs; values with X or Z
/// digits are `None`.
#[pyfunction]
fn parse_readmemh<'py>(py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyList>> {
    let words = rivet_kit::memory::parse_readmemh(text).map_err(err)?;
    let list = PyList::empty(py);
    for (i, w) in words {
        let v: Option<u128> = w.to_u128().ok();
        list.append((i, v))?;
    }
    Ok(list)
}

/// Add the kit's classes and functions to the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Memory>()?;
    m.add_class::<Rng>()?;
    m.add_class::<Scoreboard>()?;
    m.add_class::<Bins>()?;
    m.add_class::<CoverPoint>()?;
    m.add_class::<Cross>()?;
    m.add_class::<Covergroup>()?;
    m.add_function(wrap_pyfunction!(seed_for_test, m)?)?;
    m.add_function(wrap_pyfunction!(coverage_percent, m)?)?;
    m.add_function(wrap_pyfunction!(coverage_json, m)?)?;
    m.add_function(wrap_pyfunction!(coverage_table, m)?)?;
    m.add_function(wrap_pyfunction!(write_coverage, m)?)?;
    m.add_function(wrap_pyfunction!(clear_coverage, m)?)?;
    m.add_function(wrap_pyfunction!(parse_readmemh, m)?)?;
    Ok(())
}
