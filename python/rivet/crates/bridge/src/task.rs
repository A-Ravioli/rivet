//! Driving Python coroutines on Rivet's executor.
//!
//! This is the whole trick. A Python `async def` is a coroutine object:
//! `send()` runs it to its next `yield`, and our triggers yield
//! themselves. So a task is a Rust future that alternates between
//! resuming the coroutine and parking on whatever the coroutine asked
//! for — and the thing it parks on is an ordinary Rivet trigger, backed
//! by a simulator callback.
//!
//! What that buys, compared with an interpreter that owns the scheduler:
//! the scheduler, the trigger registry and the value path are all native,
//! and the interpreter is entered exactly once per `await` rather than
//! several times. What it costs is that one entry — which is why
//! [`crate::trigger::EdgeN`] exists, and why clocks, monitors and bus
//! drivers are Rust tasks that never enter Python at all.

use pyo3::exceptions::{PyRuntimeError, PyStopIteration, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::PyTraceback;
use rivet_core::error::Error;
use rivet_core::executor::WaitOn;
use rivet_core::runtime;
use rivet_core::task::JoinHandle;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use crate::trigger::{Pending, PyTrigger, Resolved, TriggerKind};

pyo3::create_exception!(rivet, SkipTest, pyo3::exceptions::PyException, "Skip the running test.");
pyo3::create_exception!(rivet, TestFailure, pyo3::exceptions::PyAssertionError, "Fail the running test.");
pyo3::create_exception!(rivet, TestSuccess, pyo3::exceptions::PyException, "End the running test as passed.");
pyo3::create_exception!(rivet, CancelledError, pyo3::exceptions::PyException, "The task was cancelled.");

/// Shared state of a spawned task.
pub struct TaskState {
    inner: RefCell<Inner>,
}

struct Inner {
    name: String,
    result: Option<Result<Py<PyAny>, PyErr>>,
    cancelled: bool,
    wakers: Vec<Waker>,
    handle: Option<JoinHandle<()>>,
    /// Whether an exception escaping this task fails the test. Off for
    /// tasks the library starts on the caller's behalf, where the caller
    /// sees the exception itself.
    report: bool,
}

impl TaskState {
    fn new(name: String, report: bool) -> TaskState {
        TaskState {
            inner: RefCell::new(Inner {
                name,
                result: None,
                cancelled: false,
                wakers: Vec::new(),
                handle: None,
                report,
            }),
        }
    }

    pub fn is_done(&self) -> bool {
        self.inner.borrow().result.is_some()
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.borrow().cancelled
    }

    pub fn name(&self) -> String {
        self.inner.borrow().name.clone()
    }

    pub fn wake_me(&self, w: Waker) {
        self.inner.borrow_mut().wakers.push(w);
    }

    fn set_handle(&self, h: JoinHandle<()>) {
        self.inner.borrow_mut().handle = Some(h);
    }

    fn complete(&self, out: Result<Py<PyAny>, PyErr>) {
        let (wakers, report, name) = {
            let mut g = self.inner.borrow_mut();
            if g.result.is_some() {
                return;
            }
            g.result = Some(out);
            (std::mem::take(&mut g.wakers), g.report, g.name.clone())
        };
        // An exception that escapes a started task fails the test, as it
        // does in cocotb and as a panicking Rust task does here.
        if report {
            let failed = {
                let g = self.inner.borrow();
                match g.result.as_ref() {
                    Some(Err(e)) => Some(Python::with_gil(|py| format_exception(py, e))),
                    _ => None,
                }
            };
            if let Some(msg) = failed {
                runtime::report_failure(format!("task {name:?} raised: {msg}"));
            }
        }
        for w in wakers {
            w.wake();
        }
    }

    /// Stop the task. Its coroutine is dropped, which deregisters whatever
    /// trigger it was parked on.
    pub fn cancel(&self) {
        let handle = {
            let mut g = self.inner.borrow_mut();
            if g.result.is_some() {
                return;
            }
            g.cancelled = true;
            g.handle.take()
        };
        if let Some(h) = handle {
            h.cancel();
        }
        let wakers = {
            let mut g = self.inner.borrow_mut();
            if g.result.is_none() {
                g.result = Some(Err(CancelledError::new_err(format!("task {:?} was cancelled", g.name))));
            }
            std::mem::take(&mut g.wakers)
        };
        for w in wakers {
            w.wake();
        }
    }

    /// The outcome, once done: the returned value, or the exception.
    pub fn take_outcome(&self) -> Option<Result<Py<PyAny>, PyErr>> {
        let g = self.inner.borrow();
        g.result.as_ref().map(|r| match r {
            Ok(v) => Python::with_gil(|py| Ok(v.clone_ref(py))),
            Err(e) => Err(Python::with_gil(|py| e.clone_ref(py))),
        })
    }
}

/// What to hand the coroutine on its next resume.
enum Resume {
    Send(Option<Py<PyAny>>),
    Throw(PyErr),
}

/// A Python coroutine, running as a Rivet task.
pub struct PyCoroDriver {
    coro: Py<PyAny>,
    pending: Option<Pending>,
    resume: Option<Resume>,
    done: bool,
}

impl PyCoroDriver {
    pub fn new(coro: Py<PyAny>) -> PyCoroDriver {
        PyCoroDriver { coro, pending: None, resume: Some(Resume::Send(None)), done: false }
    }
}

enum Step {
    /// The coroutine awaited a trigger.
    Awaited(TriggerKind),
    /// The coroutine returned.
    Returned(Py<PyAny>),
    /// The coroutine raised.
    Raised(PyErr),
}

impl Future for PyCoroDriver {
    type Output = Result<Py<PyAny>, PyErr>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(!this.done, "a finished coroutine was polled again");
        loop {
            // Park on whatever the last resume asked for.
            if let Some(p) = this.pending.as_mut() {
                match p.poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(r) => {
                        this.pending = None;
                        this.resume = Some(Python::with_gil(|py| resolved_to_resume(py, r)));
                    }
                }
            }

            let step = Python::with_gil(|py| resume(py, &this.coro, this.resume.take()));
            match step {
                Step::Awaited(kind) => match kind.arm() {
                    Ok(p) => this.pending = Some(p),
                    // The trigger could not be armed (a zero-length
                    // Timer, say). Raise it inside the coroutine so the
                    // traceback points at the `await` that asked for it.
                    Err(e) => this.resume = Some(Resume::Throw(e)),
                },
                Step::Returned(v) => {
                    this.done = true;
                    return Poll::Ready(Ok(v));
                }
                Step::Raised(e) => {
                    this.done = true;
                    return Poll::Ready(Err(e));
                }
            }
        }
    }
}

fn resolved_to_resume(py: Python<'_>, r: Resolved) -> Resume {
    match r {
        Resolved::Nothing => Resume::Send(None),
        Resolved::Index(i) => Resume::Send(Some(i.into_pyobject(py).unwrap().into_any().unbind())),
        Resolved::Joined(state) => match state.take_outcome() {
            Some(Ok(v)) => Resume::Send(Some(v)),
            Some(Err(e)) => Resume::Throw(e),
            None => Resume::Send(None),
        },
    }
}

/// One resume of the coroutine. This is the only place the interpreter is
/// entered on the hot path.
fn resume(py: Python<'_>, coro: &Py<PyAny>, how: Option<Resume>) -> Step {
    let out = match how {
        Some(Resume::Throw(e)) => coro.call_method1(py, pyo3::intern!(py, "throw"), (e,)),
        Some(Resume::Send(Some(v))) => coro.call_method1(py, pyo3::intern!(py, "send"), (v,)),
        _ => coro.call_method1(py, pyo3::intern!(py, "send"), (py.None(),)),
    };
    match out {
        Ok(obj) => match obj.extract::<PyTrigger>(py) {
            Ok(t) => Step::Awaited(t.kind),
            Err(_) => Step::Raised(PyTypeError::new_err(format!(
                "awaited {}, which is not a Rivet trigger — a Rivet test can only await Rivet \
                 triggers and tasks (asyncio objects have no simulator to wait on)",
                obj.bind(py).get_type().name().map(|n| n.to_string()).unwrap_or_else(|_| "?".into())
            ))),
        },
        Err(e) if e.is_instance_of::<PyStopIteration>(py) => {
            // A coroutine that returns raises StopIteration carrying the
            // value; `None` for the usual test that returns nothing.
            let value =
                e.value(py).getattr(pyo3::intern!(py, "value")).map(|v| v.unbind()).unwrap_or_else(|_| py.None());
            Step::Returned(value)
        }
        Err(e) => Step::Raised(e),
    }
}

/// Start `coro` as a task that runs alongside the caller.
pub fn spawn(coro: Py<PyAny>, name: String, report: bool) -> PyResult<Rc<TaskState>> {
    crate::runtime_guard::ensure_running()?;
    let state = Rc::new(TaskState::new(name.clone(), report));
    let sink = state.clone();
    let handle = rivet_core::task::spawn_named(&name, async move {
        let out = PyCoroDriver::new(coro).await;
        sink.complete(out);
    });
    state.set_handle(handle);
    Ok(state)
}

/// A started task.
#[pyclass(name = "Task", module = "rivet", unsendable)]
pub struct PyTask {
    pub state: Rc<TaskState>,
}

#[pymethods]
impl PyTask {
    /// Await the task's return value. Re-raises whatever it raised.
    fn join(&self) -> PyTrigger {
        PyTrigger::new(TriggerKind::Join { state: self.state.clone() })
    }

    /// Await the task. `await task` is `await task.join()`.
    fn __await__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let t = self.join();
        let bound = t.into_pyobject(py)?;
        bound.call_method0("__await__").map(|o| o.unbind())
    }

    /// Stop the task, dropping whatever it was waiting on.
    fn cancel(&self) {
        self.state.cancel();
    }

    #[getter]
    fn done(&self) -> bool {
        self.state.is_done()
    }

    #[getter]
    fn cancelled(&self) -> bool {
        self.state.is_cancelled()
    }

    #[getter]
    fn name(&self) -> String {
        self.state.name()
    }

    /// The return value, if the task has finished. Raises if it raised.
    fn result(&self) -> PyResult<PyObject> {
        match self.state.take_outcome() {
            Some(Ok(v)) => Ok(v),
            Some(Err(e)) => Err(e),
            None => Err(PyRuntimeError::new_err(format!("task {:?} has not finished", self.state.name()))),
        }
    }

    fn __repr__(&self) -> String {
        let s = if self.state.is_cancelled() {
            "cancelled"
        } else if self.state.is_done() {
            "done"
        } else {
            "running"
        };
        format!("<Task {:?} {s}>", self.state.name())
    }
}

/// Note what a task is waiting on, so a hang names itself.
pub fn note(w: WaitOn) {
    runtime::note_wait(w);
}

/// Render an exception the way Python would print it, traceback included,
/// so a failure in `results.xml` says where it happened.
pub fn format_exception(py: Python<'_>, e: &PyErr) -> String {
    let mut out = String::new();
    if let Some(tb) = e.traceback(py) {
        if let Ok(Ok(text)) = tb.downcast::<PyTraceback>().map(|t| t.format()) {
            out.push_str(&text);
        }
    }
    let value = e.value(py);
    let type_name = value.get_type().name().map(|n| n.to_string()).unwrap_or_else(|_| "Exception".into());
    let msg = value.str().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if msg.is_empty() {
        out.push_str(&type_name);
    } else {
        out.push_str(&format!("{type_name}: {msg}"));
    }
    out
}

/// Map a Python exception onto the outcome the regression loop records.
pub fn py_err_to_rivet(py: Python<'_>, e: PyErr) -> Error {
    if e.is_instance_of::<SkipTest>(py) {
        return Error::Skip(e.value(py).str().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    }
    Error::Msg(format_exception(py, &e))
}
