//! Awaitable simulation events.
//!
//! Every trigger is a plain `Future`. The first poll registers with the
//! runtime (cocotb's `_prime`); dropping the future before it fires
//! deregisters (cocotb's `_unprime`).

use crate::backend::{CbId, CbKind, Handle};
use crate::error::Error;
use crate::runtime::{self, EdgeKind, Phase};
use crate::time::Duration;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Fires after a simulated duration; returns at the beginning of that time
/// step.
pub struct Timer {
    steps: u64,
    cb: Option<CbId>,
    fired: bool,
}

impl Timer {
    pub fn new(d: Duration) -> Timer {
        let steps = d.to_steps(runtime::precision());
        Timer::steps(steps.max(1))
    }

    pub fn steps(steps: u64) -> Timer {
        assert!(steps > 0, "Timer must advance time");
        Timer { steps, cb: None, fired: false }
    }
}

impl Future for Timer {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.fired {
            return Poll::Ready(());
        }
        match self.cb {
            None => {
                let id = runtime::with(|rt| rt.add_timer(self.steps, cx.waker().clone()));
                self.cb = Some(id);
                Poll::Pending
            }
            Some(id) => {
                let pending = runtime::with(|rt| rt.timers_contains(id));
                if pending {
                    runtime::with(|rt| rt.update_timer_waker(id, cx.waker().clone()));
                    Poll::Pending
                } else {
                    self.fired = true;
                    self.cb = None;
                    Poll::Ready(())
                }
            }
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(id) = self.cb.take() {
            if !self.fired && runtime::is_initialised() {
                runtime::with(|rt| rt.remove_timer(id));
            }
        }
    }
}

/// Edge trigger on a scalar signal.
pub struct Edge {
    handle: Handle,
    kind: EdgeKind,
    waiter: Option<u64>,
    fired: bool,
}

impl Edge {
    pub fn new(handle: Handle, kind: EdgeKind) -> Edge {
        Edge { handle, kind, waiter: None, fired: false }
    }
}

impl Future for Edge {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.fired {
            return Poll::Ready(());
        }
        match self.waiter {
            None => {
                let (h, k) = (self.handle, self.kind);
                let id = runtime::with(|rt| rt.add_edge_waiter(h, k, cx.waker().clone()));
                self.waiter = Some(id);
                Poll::Pending
            }
            Some(id) => {
                let h = self.handle;
                let still_waiting = runtime::with(|rt| rt.edge_waiter_pending(h, id, cx.waker()));
                if still_waiting {
                    Poll::Pending
                } else {
                    self.fired = true;
                    self.waiter = None;
                    Poll::Ready(())
                }
            }
        }
    }
}

impl Drop for Edge {
    fn drop(&mut self) {
        if let Some(id) = self.waiter.take() {
            if !self.fired && runtime::is_initialised() {
                let h = self.handle;
                runtime::with(|rt| rt.remove_edge_waiter(h, id));
            }
        }
    }
}

/// A phase trigger: `ReadWrite`, `ReadOnly`, or `NextTimeStep`.
pub struct PhaseTrigger {
    kind: CbKind,
    registered: bool,
    gen: u64,
}

impl PhaseTrigger {
    fn new(kind: CbKind) -> PhaseTrigger {
        PhaseTrigger { kind, registered: false, gen: 0 }
    }
}

impl Future for PhaseTrigger {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if !self.registered {
            let phase = runtime::phase();
            if phase == Phase::EndTimeStep && matches!(self.kind, CbKind::ReadWrite | CbKind::ReadOnly) {
                panic!("illegal transition: awaiting {:?} in the ReadOnly phase", self.kind);
            }
            let kind = self.kind;
            self.gen = runtime::with(|rt| rt.add_phase_waiter(kind, cx.waker().clone()));
            self.registered = true;
            return Poll::Pending;
        }
        let kind = self.kind;
        let gen = self.gen;
        if runtime::with(|rt| rt.phase_fired(kind, gen)) {
            Poll::Ready(())
        } else {
            runtime::with(|rt| rt.add_phase_waiter_existing(kind, cx.waker().clone()));
            Poll::Pending
        }
    }
}

/// Fires at the end of the current evaluation cycle, when values have
/// settled. Writes are still allowed.
pub fn read_write() -> PhaseTrigger {
    PhaseTrigger::new(CbKind::ReadWrite)
}

/// Fires at the end of the current time step. No writes allowed until the
/// next time step.
pub fn read_only() -> PhaseTrigger {
    PhaseTrigger::new(CbKind::ReadOnly)
}

/// Fires at the beginning of the next time step.
pub fn next_time_step() -> PhaseTrigger {
    PhaseTrigger::new(CbKind::NextTimeStep)
}

pub fn rising_edge(h: Handle) -> Edge {
    Edge::new(h, EdgeKind::Rising)
}
pub fn falling_edge(h: Handle) -> Edge {
    Edge::new(h, EdgeKind::Falling)
}
pub fn value_change(h: Handle) -> Edge {
    Edge::new(h, EdgeKind::Any)
}

/// Yield once so other ready tasks run; no simulation time passes.
pub struct YieldNow(bool);

pub fn yield_now() -> YieldNow {
    YieldNow(false)
}

impl Future for YieldNow {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

// ---------------------------------------------------------------------------
// Combinators

pub enum Either<A, B> {
    Left(A),
    Right(B),
}

pin_project_lite_free::pin2! {
    /// Resolves with whichever of two futures completes first; the other is
    /// dropped.
    pub struct First<A, B> { a: A, b: B }
}

impl<A: Future, B: Future> Future for First<A, B> {
    type Output = Either<A::Output, B::Output>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (a, b) = self.project();
        if let Poll::Ready(v) = a.poll(cx) {
            return Poll::Ready(Either::Left(v));
        }
        if let Poll::Ready(v) = b.poll(cx) {
            return Poll::Ready(Either::Right(v));
        }
        Poll::Pending
    }
}

pub fn first<A: Future, B: Future>(a: A, b: B) -> First<A, B> {
    First { a, b }
}

pin_project_lite_free::pin2! {
    /// Resolves when both futures have completed.
    pub struct Join<A: Future, B: Future> { a: MaybeDone<A>, b: MaybeDone<B> }
}

pub enum MaybeDone<F: Future> {
    Pending(F),
    Done(Option<F::Output>),
}

impl<F: Future> MaybeDone<F> {
    fn poll_done(self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool {
        // SAFETY: we never move the inner future out while pinned.
        let this = unsafe { self.get_unchecked_mut() };
        match this {
            MaybeDone::Pending(f) => {
                let f = unsafe { Pin::new_unchecked(f) };
                match f.poll(cx) {
                    Poll::Ready(v) => {
                        *this = MaybeDone::Done(Some(v));
                        true
                    }
                    Poll::Pending => false,
                }
            }
            MaybeDone::Done(_) => true,
        }
    }
    fn take(&mut self) -> F::Output {
        match self {
            MaybeDone::Done(v) => v.take().expect("taken twice"),
            _ => unreachable!(),
        }
    }
}

impl<A: Future, B: Future> Future for Join<A, B> {
    type Output = (A::Output, B::Output);
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (da, db) = {
            let (a, b) = self.as_mut().project();
            (a.poll_done(cx), b.poll_done(cx))
        };
        if da && db {
            // SAFETY: both halves are `Done`, which holds no pinned future.
            let this = unsafe { self.get_unchecked_mut() };
            Poll::Ready((this.a.take(), this.b.take()))
        } else {
            Poll::Pending
        }
    }
}

pub fn join<A: Future, B: Future>(a: A, b: B) -> Join<A, B> {
    Join { a: MaybeDone::Pending(a), b: MaybeDone::Pending(b) }
}

/// Run `fut` with a simulated-time timeout.
pub async fn with_timeout<F: Future>(fut: F, d: Duration) -> Result<F::Output, Error> {
    match first(fut, Timer::new(d)).await {
        Either::Left(v) => Ok(v),
        Either::Right(()) => Err(Error::Timeout(format!("{d}"))),
    }
}

/// Minimal structural pin projection for two-field structs, to avoid a
/// dependency.
mod pin_project_lite_free {
    macro_rules! pin2 {
        ($(#[$m:meta])* pub struct $name:ident<$($g:ident $(: $b:path)?),*> { $fa:ident: $ta:ty, $fb:ident: $tb:ty }) => {
            $(#[$m])*
            pub struct $name<$($g $(: $b)?),*> { $fa: $ta, $fb: $tb }
            impl<$($g $(: $b)?),*> $name<$($g),*> {
                #[inline]
                fn project(self: ::std::pin::Pin<&mut Self>) -> (::std::pin::Pin<&mut $ta>, ::std::pin::Pin<&mut $tb>) {
                    // SAFETY: both fields are structurally pinned; the struct
                    // has no Drop impl and is never unpinned by other code.
                    unsafe {
                        let this = self.get_unchecked_mut();
                        (::std::pin::Pin::new_unchecked(&mut this.$fa), ::std::pin::Pin::new_unchecked(&mut this.$fb))
                    }
                }
            }
        };
    }
    pub(crate) use pin2;
}
