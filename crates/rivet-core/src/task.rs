//! Spawning and joining tasks.

use crate::error::Error;
use crate::executor::TaskKey;
use crate::runtime;
use std::cell::RefCell;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

struct JoinInner<T> {
    result: Option<Result<T, Error>>,
    waker: Option<Waker>,
}

/// A handle to a spawned task. Dropping it detaches the task; use
/// [`JoinHandle::cancel`] to stop it, or `.await` it to get its output.
pub struct JoinHandle<T> {
    inner: Rc<RefCell<JoinInner<T>>>,
    key: TaskKey,
    name: String,
}

struct SpawnedTask<F: Future> {
    fut: F,
    inner: Rc<RefCell<JoinInner<F::Output>>>,
    name: String,
}

impl<F: Future> Future for SpawnedTask<F> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // SAFETY: structural pinning of `fut`; nothing else moves it.
        let this = unsafe { self.get_unchecked_mut() };
        let fut = unsafe { Pin::new_unchecked(&mut this.fut) };
        match catch_unwind(AssertUnwindSafe(|| fut.poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(v)) => {
                let w = {
                    let mut inner = this.inner.borrow_mut();
                    inner.result = Some(Ok(v));
                    inner.waker.take()
                };
                if let Some(w) = w {
                    w.wake();
                }
                Poll::Ready(())
            }
            Err(payload) => {
                let msg = runtime::panic_message(&payload);
                log::error!("task {:?} panicked: {msg}", this.name);
                let w = {
                    let mut inner = this.inner.borrow_mut();
                    inner.result = Some(Err(Error::Panicked(msg.clone())));
                    inner.waker.take()
                };
                if let Some(w) = w {
                    w.wake();
                }
                runtime::report_failure(format!("task {:?} panicked: {msg}", this.name));
                Poll::Ready(())
            }
        }
    }
}

/// Spawn a task that runs concurrently with the current one. Its first
/// poll happens the next time the executor runs, in FIFO order.
pub fn spawn<F>(fut: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
{
    spawn_named("task", fut)
}

pub fn spawn_named<F>(name: &str, fut: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
{
    let inner = Rc::new(RefCell::new(JoinInner { result: None, waker: None }));
    let task = SpawnedTask { fut, inner: inner.clone(), name: name.to_string() };
    let key = runtime::spawn_raw(name.to_string(), Box::pin(task));
    JoinHandle { inner, key, name: name.to_string() }
}

impl<T> JoinHandle<T> {
    /// Stop the task. Its future is dropped, running destructors, which
    /// deregisters any triggers it was waiting on. Awaiting the handle
    /// afterwards yields `Err(Cancelled)`.
    pub fn cancel(&self) {
        if runtime::task_alive(self.key) {
            runtime::cancel_task(self.key);
            let w = {
                let mut inner = self.inner.borrow_mut();
                if inner.result.is_none() {
                    inner.result = Some(Err(Error::Cancelled));
                }
                inner.waker.take()
            };
            if let Some(w) = w {
                w.wake();
            }
        }
    }

    pub fn is_done(&self) -> bool {
        self.inner.borrow().result.is_some()
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.inner.borrow_mut();
        match inner.result.take() {
            Some(r) => Poll::Ready(r),
            None => {
                inner.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// Structured concurrency: tasks spawned in a scope are cancelled when the
/// scope is dropped (normally or by `?`).
#[derive(Default)]
pub struct Scope {
    handles: Vec<Box<dyn CancelHandle>>,
}

trait CancelHandle {
    fn cancel_it(&self);
}
impl<T> CancelHandle for JoinHandle<T> {
    fn cancel_it(&self) {
        self.cancel();
    }
}

impl Scope {
    pub fn new() -> Scope {
        Scope::default()
    }

    /// Spawn a child task owned by this scope.
    pub fn spawn<F>(&mut self, fut: F) -> JoinHandleRef
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let h = spawn(fut);
        let r = JoinHandleRef { inner_done: Rc::new(RefCell::new(None)), key: h.key };
        let done = r.inner_done.clone();
        let inner = h.inner.clone();
        // Bridge completion without keeping T generic on the scope.
        let watcher = spawn(async move {
            let res: Result<(), Error> = loop {
                let taken = {
                    let mut g = inner.borrow_mut();
                    match g.result.take() {
                        Some(Ok(_)) => Some(Ok(())),
                        Some(Err(e)) => Some(Err(e)),
                        None => None,
                    }
                };
                if let Some(r) = taken {
                    break r;
                }
                WaitOn { inner: inner.clone() }.await;
            };
            *done.borrow_mut() = Some(res);
        });
        self.handles.push(Box::new(h));
        self.handles.push(Box::new(watcher));
        r
    }
}

struct WaitOn<T> {
    inner: Rc<RefCell<JoinInner<T>>>,
}
impl<T> Future for WaitOn<T> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut g = self.inner.borrow_mut();
        if g.result.is_some() {
            Poll::Ready(())
        } else {
            g.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Lightweight status view of a scoped task.
pub struct JoinHandleRef {
    inner_done: Rc<RefCell<Option<Result<(), Error>>>>,
    key: TaskKey,
}

impl JoinHandleRef {
    pub fn is_done(&self) -> bool {
        self.inner_done.borrow().is_some()
    }
    pub fn is_alive(&self) -> bool {
        runtime::task_alive(self.key)
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        for h in &self.handles {
            h.cancel_it();
        }
    }
}
