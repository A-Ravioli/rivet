//! Awaitable simulation events.
//!
//! A trigger is a description, not a registration: constructing one costs
//! an enum write. It is registered with the simulator when the driver
//! actually parks on it, and deregistered by dropping it — the same
//! prime/unprime discipline the Rust triggers use, without a Python object
//! per callback.
//!
//! `__await__` yields the trigger itself once. The driver
//! ([`crate::task`]) receives it, turns it into a Rust future, and sends
//! the result of that future back into the coroutine.

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rivet_core::backend::{CbKind, Handle};
use rivet_core::runtime::EdgeKind;
use rivet_core::time::Duration;
use rivet_core::triggers::{next_time_step, read_only, read_write, yield_now, Edge, PhaseTrigger, Timer, YieldNow};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::task::TaskState;

/// What a trigger waits for.
#[derive(Clone)]
pub enum TriggerKind {
    Timer {
        duration: Duration,
    },
    Edge {
        handle: Handle,
        kind: EdgeKind,
        count: u64,
    },
    Phase {
        kind: CbKind,
    },
    Yield,
    Join {
        state: Rc<TaskState>,
    },
    /// Whichever of these fires first; the rest are dropped.
    First {
        of: Vec<TriggerKind>,
    },
}

/// An awaitable handed to `await`.
#[pyclass(name = "Trigger", module = "rivet", unsendable)]
#[derive(Clone)]
pub struct PyTrigger {
    pub kind: TriggerKind,
}

impl PyTrigger {
    pub fn new(kind: TriggerKind) -> PyTrigger {
        PyTrigger { kind }
    }
}

#[pymethods]
impl PyTrigger {
    /// Yield this trigger to the driver, once.
    fn __await__(slf: PyRef<'_, Self>) -> TriggerIter {
        TriggerIter { trigger: Some(slf.clone()) }
    }

    /// The task this trigger joins, or `None` for any other trigger.
    ///
    /// `with_timeout` uses it: awaiting `task.join()` with a limit should
    /// still give back what the task returned, and a bare trigger has no
    /// value to give.
    #[getter]
    fn task(&self) -> Option<crate::task::PyTask> {
        match &self.kind {
            TriggerKind::Join { state } => Some(crate::task::PyTask { state: state.clone() }),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        format!("<Trigger {}>", describe(&self.kind))
    }
}

fn describe(k: &TriggerKind) -> String {
    match k {
        TriggerKind::Timer { duration } => format!("Timer({duration})"),
        TriggerKind::Edge { kind, count, .. } => {
            let name = match kind {
                EdgeKind::Rising => "RisingEdge",
                EdgeKind::Falling => "FallingEdge",
                EdgeKind::Any => "ValueChange",
            };
            if *count == 1 {
                name.to_string()
            } else {
                format!("{name} x{count}")
            }
        }
        TriggerKind::Phase { kind } => format!("{kind:?}"),
        TriggerKind::Yield => "yield".into(),
        TriggerKind::Join { .. } => "Join".into(),
        TriggerKind::First { of } => {
            format!("First({})", of.iter().map(describe).collect::<Vec<_>>().join(", "))
        }
    }
}

/// The one-shot iterator `__await__` returns.
///
/// It has to implement the whole generator protocol, not just
/// `__next__`. When a coroutine is suspended at `await trigger` and
/// something calls `coro.send(value)` with a value that is not `None`,
/// CPython forwards that `send` to whatever the `await` is delegating
/// to — this object. Answering it with `StopIteration(value)` is what
/// makes the `await` expression evaluate to `value`, which is how
/// `first()` reports its winner and how a joined task returns its
/// result. The same goes for `throw`, which is how a joined task's
/// exception is raised at the `await` that was waiting for it.
#[pyclass(module = "rivet", unsendable)]
pub struct TriggerIter {
    trigger: Option<PyTrigger>,
}

#[pymethods]
impl TriggerIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Yield the trigger once, then stop.
    fn __next__(&mut self) -> Option<PyTrigger> {
        self.trigger.take()
    }

    /// The first `send` is the one that starts the await and is always
    /// `None`; a later one carries the value the await evaluates to.
    fn send(&mut self, value: Py<PyAny>) -> PyResult<PyTrigger> {
        match self.trigger.take() {
            Some(t) => Ok(t),
            None => Err(pyo3::exceptions::PyStopIteration::new_err(value)),
        }
    }

    /// Raise inside the awaiting coroutine, at the `await`.
    #[pyo3(signature = (*args))]
    fn throw(&mut self, args: &Bound<'_, pyo3::types::PyTuple>) -> PyResult<()> {
        self.trigger = None;
        match args.get_item(0) {
            Ok(e) => Err(PyErr::from_value(e)),
            Err(_) => Err(PyRuntimeError::new_err("throw() needs an exception")),
        }
    }

    fn close(&mut self) {
        self.trigger = None;
    }
}

// ---------------------------------------------------------------------------
// The Rust side: a trigger in flight.

/// An armed trigger. Boxing is avoided: every variant is a concrete,
/// `Unpin` future, so parking on one costs no allocation.
pub enum Pending {
    Timer(Timer),
    Edge(EdgeN),
    Phase(PhaseTrigger),
    Yield(YieldNow),
    Join(Rc<TaskState>),
    First(Vec<Pending>),
}

impl TriggerKind {
    /// Arm this trigger. Needs a live runtime (for the time precision).
    pub fn arm(&self) -> PyResult<Pending> {
        Ok(match self {
            TriggerKind::Timer { duration } => {
                let precision = crate::runtime_guard::precision()?;
                let steps = duration.to_steps(precision);
                if steps == 0 {
                    return Err(PyRuntimeError::new_err(format!(
                        "{duration} is shorter than one simulator step — it would not advance time"
                    )));
                }
                Pending::Timer(Timer::steps(steps))
            }
            TriggerKind::Edge { handle, kind, count } => {
                Pending::Edge(EdgeN { handle: *handle, kind: *kind, left: *count, current: Edge::new(*handle, *kind) })
            }
            TriggerKind::Phase { kind } => Pending::Phase(match kind {
                CbKind::ReadWrite => read_write(),
                CbKind::ReadOnly => read_only(),
                _ => next_time_step(),
            }),
            TriggerKind::Yield => Pending::Yield(yield_now()),
            TriggerKind::Join { state } => Pending::Join(state.clone()),
            TriggerKind::First { of } => Pending::First(of.iter().map(TriggerKind::arm).collect::<PyResult<Vec<_>>>()?),
        })
    }
}

/// The value an `await` evaluates to.
pub enum Resolved {
    /// `None` in Python.
    Nothing,
    /// The index of the trigger that won a `first()`.
    Index(usize),
    /// A joined task's return value, or its failure.
    Joined(Rc<TaskState>),
}

impl Pending {
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Resolved> {
        match self {
            Pending::Timer(t) => Pin::new(t).poll(cx).map(|()| Resolved::Nothing),
            Pending::Edge(e) => Pin::new(e).poll(cx).map(|()| Resolved::Nothing),
            Pending::Phase(p) => Pin::new(p).poll(cx).map(|()| Resolved::Nothing),
            Pending::Yield(y) => Pin::new(y).poll(cx).map(|()| Resolved::Nothing),
            Pending::Join(state) => {
                if state.is_done() {
                    Poll::Ready(Resolved::Joined(state.clone()))
                } else {
                    state.wake_me(cx.waker().clone());
                    rivet_core::runtime::note_wait(rivet_core::executor::WaitOn::Join);
                    Poll::Pending
                }
            }
            Pending::First(all) => {
                for (i, p) in all.iter_mut().enumerate() {
                    if p.poll(cx).is_ready() {
                        return Poll::Ready(Resolved::Index(i));
                    }
                }
                Poll::Pending
            }
        }
    }
}

/// `n` edges of one signal, awaited without returning to Python between
/// them. `await clk.rising_edge(n=1000)` is one coroutine resume, not a
/// thousand — which is what makes a "skip past the boring part" loop cost
/// the same in Python as in Rust.
pub struct EdgeN {
    handle: Handle,
    kind: EdgeKind,
    left: u64,
    current: Edge,
}

impl Future for EdgeN {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        loop {
            match Pin::new(&mut this.current).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(()) => {
                    this.left -= 1;
                    if this.left == 0 {
                        return Poll::Ready(());
                    }
                    this.current = Edge::new(this.handle, this.kind);
                }
            }
        }
    }
}
