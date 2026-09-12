//! The runtime: owns the backend and the executor, tracks the timestep
//! phase, buffers writes, and turns simulator callbacks into task wakeups.
//!
//! There is one runtime per process, in a thread-local. The simulator's
//! thread is the only thread that may touch it. Backends deliver events
//! through [`dispatch`]; everything else in the crate reaches the runtime
//! through [`with`].

use crate::backend::{Action, Backend, Capabilities, CbId, CbKind, Handle, OwnedValue, Value};
use crate::executor::{Executor, TaskKey};
use crate::value::{Logic, LogicVec};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

/// Where in the timestep the harness currently is. See
/// `docs/design/01-architecture.md` §5 and cocotb's timing model.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Before the simulation started.
    Startup,
    /// `Timer` or `NextTimeStep` just returned.
    BeginTimeStep,
    /// An edge trigger just returned; downstream HDL has not reacted yet.
    ValuesChange,
    /// `ReadWrite` just returned; values are settled, writes are allowed.
    ValuesSettle,
    /// `ReadOnly` just returned; no writes allowed.
    EndTimeStep,
}

/// Events a backend delivers to the runtime.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Event {
    StartOfSim,
    EndOfSim,
    ValueChange(Handle),
    Timer(CbId),
    ReadWrite,
    ReadOnly,
    NextTimeStep,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum EdgeKind {
    Rising,
    Falling,
    Any,
}

struct EdgeWaiter {
    id: u64,
    kind: EdgeKind,
    waker: Waker,
}

struct EdgeReg {
    _cb: CbId,
    waiters: Vec<EdgeWaiter>,
    scratch: LogicVec,
}

#[derive(Default)]
struct PhaseWaiters {
    cb: Option<CbId>,
    waiters: Vec<Waker>,
    /// Incremented each time the phase fires, so a waiter can tell whether
    /// the firing it registered for has happened.
    gen: u64,
}

struct PendingWrite {
    handle: Handle,
    value: OwnedValue,
}

pub(crate) struct CurrentTest {
    pub failure: Option<String>,
    pub waker: Option<Waker>,
}

pub struct Runtime {
    pub(crate) backend: Box<dyn Backend>,
    pub(crate) exec: Executor,
    pub(crate) caps: Capabilities,
    pub(crate) phase: Phase,
    timers: HashMap<CbId, Waker>,
    edges: HashMap<Handle, EdgeReg>,
    next_waiter_id: u64,
    rw: PhaseWaiters,
    ro: PhaseWaiters,
    nts: PhaseWaiters,
    writes: Vec<PendingWrite>,
    write_index: HashMap<Handle, usize>,
    pub(crate) root: Option<Handle>,
    pub(crate) current_test: Option<CurrentTest>,
    pub(crate) sim_ended: bool,
    pub(crate) finished: bool,
    /// Process exit code the backend should use where it can.
    pub exit_code: i32,
    /// Called once at StartOfSim to spawn the top-level task.
    entry: Option<Box<dyn FnOnce()>>,
    /// Called at EndOfSim if the simulator ends before `finish()`.
    on_premature_end: Option<Box<dyn FnOnce()>>,
}

thread_local! {
    static RT: RefCell<Option<Runtime>> = const { RefCell::new(None) };
    /// Set while a simulator event is being handled. Lives outside `RT` so
    /// a simulator that fires a callback synchronously from inside a
    /// `vpi_put_value` (Icarus, Xcelium, Questa) can be answered while the
    /// runtime is still borrowed by the write.
    static REACTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DEFERRED: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
}

/// Install the runtime. Called once by the backend's startup routine.
pub fn init(backend: Box<dyn Backend>) {
    let caps = backend.caps();
    let rt = Runtime {
        backend,
        exec: Executor::new(),
        caps,
        phase: Phase::Startup,
        timers: HashMap::new(),
        edges: HashMap::new(),
        next_waiter_id: 1,
        rw: PhaseWaiters::default(),
        ro: PhaseWaiters::default(),
        nts: PhaseWaiters::default(),
        writes: Vec::new(),
        write_index: HashMap::new(),
        root: None,
        current_test: None,
        sim_ended: false,
        finished: false,
        exit_code: 0,
        entry: None,
        on_premature_end: None,
    };
    RT.with(|cell| {
        let mut slot = cell.borrow_mut();
        assert!(slot.is_none(), "rivet runtime initialised twice");
        *slot = Some(rt);
    });
}

/// Tear down the runtime (tests and shutdown).
pub fn shutdown() -> Option<Box<dyn Backend>> {
    REACTING.with(|r| r.set(false));
    DEFERRED.with(|d| d.borrow_mut().clear());
    let rt = RT.with(|cell| cell.borrow_mut().take())?;
    // Drop tasks before the backend so trigger destructors can deregister.
    let Runtime { backend, exec, .. } = rt;
    drop(exec);
    Some(backend)
}

pub fn is_initialised() -> bool {
    RT.with(|cell| cell.try_borrow().map(|c| c.is_some()).unwrap_or(true))
}

/// `(now, precision)` if the runtime exists and is not currently borrowed;
/// safe to call from a logger.
pub fn try_time() -> Option<(u64, i32)> {
    RT.with(|cell| {
        let slot = cell.try_borrow_mut().ok()?;
        let rt = slot.as_ref()?;
        Some((rt.backend.now(), rt.backend.precision()))
    })
}

/// Borrow the runtime briefly. Panics if the runtime is missing or already
/// borrowed; callers must never hold the borrow across user code.
pub fn with<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    RT.with(|cell| {
        let mut slot = cell.try_borrow_mut().expect("rivet runtime re-entered while borrowed");
        let rt = slot.as_mut().expect("rivet runtime not initialised");
        f(rt)
    })
}

/// Set the function that spawns the top-level task at StartOfSim.
pub fn set_entry(f: impl FnOnce() + 'static) {
    with(|rt| rt.entry = Some(Box::new(f)));
}

pub fn set_premature_end_handler(f: impl FnOnce() + 'static) {
    with(|rt| rt.on_premature_end = Some(Box::new(f)));
}

/// Current simulation time in precision steps.
pub fn now() -> u64 {
    with(|rt| rt.backend.now())
}

/// Simulator time precision exponent; backends resolve it lazily since
/// it may not be known before the simulation starts.
pub fn precision() -> i32 {
    with(|rt| rt.backend.precision())
}

pub fn phase() -> Phase {
    with(|rt| rt.phase)
}

pub fn caps() -> Capabilities {
    with(|rt| rt.caps)
}

// ---------------------------------------------------------------------------
// Task management

/// Spawn a task on the executor. Returns its key.
pub(crate) fn spawn_raw(name: String, fut: Pin<Box<dyn Future<Output = ()>>>) -> TaskKey {
    with(|rt| rt.exec.spawn(name, fut))
}

pub(crate) fn cancel_task(key: TaskKey) {
    let task = with(|rt| rt.exec.cancel(key));
    drop(task);
}

pub(crate) fn task_alive(key: TaskKey) -> bool {
    with(|rt| rt.exec.is_alive(key))
}

/// Cancel every task except the current one. Used between tests.
pub fn cancel_all_other_tasks() {
    let tasks = with(|rt| {
        let keep = rt.exec.current;
        rt.exec.cancel_all_except(keep)
    });
    drop(tasks);
}

/// Drop any buffered deposits (between tests).
pub fn discard_pending_writes() {
    with(|rt| {
        rt.writes.clear();
        rt.write_index.clear();
    });
}

/// Poll ready tasks until none is runnable.
pub fn run_to_idle() {
    let mut guard = 0u64;
    loop {
        let Some((key, mut task)) = with(|rt| rt.exec.next_ready()) else { break };
        with(|rt| rt.exec.current = Some(key));
        let mut cx = Context::from_waker(&task.waker);
        let res = catch_unwind(AssertUnwindSafe(|| task.fut.as_mut().poll(&mut cx)));
        with(|rt| rt.exec.current = None);
        match res {
            Ok(Poll::Pending) => {
                let leftover = with(|rt| rt.exec.park(key, task));
                drop(leftover);
            }
            Ok(Poll::Ready(())) => {
                with(|rt| rt.exec.release(key));
                drop(task);
            }
            Err(payload) => {
                // Spawned tasks catch their own panics; reaching here means a
                // raw task panicked. Record it as a test failure.
                let msg = panic_message(&payload);
                log::error!("task {:?} panicked: {msg}", task.name);
                with(|rt| rt.exec.release(key));
                drop(task);
                report_failure(format!("task panicked: {msg}"));
            }
        }
        guard += 1;
        if guard.is_multiple_of(1_000_000) {
            log::warn!("executor ran {guard} polls without returning to the simulator; possible busy loop");
        }
    }
}

pub fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Record a failure against the running test and wake the test driver.
pub fn report_failure(msg: String) {
    let waker = with(|rt| {
        if let Some(t) = rt.current_test.as_mut() {
            if t.failure.is_none() {
                t.failure = Some(msg.clone());
            }
            t.waker.take()
        } else {
            log::error!("failure outside any test: {msg}");
            None
        }
    });
    if let Some(w) = waker {
        w.wake();
    }
}

// ---------------------------------------------------------------------------
// Event dispatch

/// Deliver a simulator event. Re-entrant calls (a simulator firing a value
/// change from inside a write) are queued and drained after the current
/// event, exactly as cocotb's VPI callback queue does.
pub fn dispatch(ev: Event) {
    if REACTING.with(|r| r.get()) {
        DEFERRED.with(|d| d.borrow_mut().push_back(ev));
        return;
    }
    REACTING.with(|r| r.set(true));
    handle_event(ev);
    run_to_idle();
    while let Some(ev) = DEFERRED.with(|d| d.borrow_mut().pop_front()) {
        handle_event(ev);
        run_to_idle();
    }
    REACTING.with(|r| r.set(false));
}

fn handle_event(ev: Event) {
    log::trace!("event {ev:?} at {}", with(|rt| rt.backend.now()));
    match ev {
        Event::StartOfSim => {
            with(|rt| rt.phase = Phase::BeginTimeStep);
            let entry = with(|rt| rt.entry.take());
            if let Some(f) = entry {
                f();
            }
        }
        Event::EndOfSim => {
            let (finished, handler) = with(|rt| {
                rt.sim_ended = true;
                (rt.finished, rt.on_premature_end.take())
            });
            if !finished {
                if let Some(f) = handler {
                    f();
                }
            }
        }
        Event::ValueChange(h) => {
            with(|rt| rt.phase = Phase::ValuesChange);
            let wakers = with(|rt| rt.collect_edge_wakers(h));
            for w in wakers {
                w.wake();
            }
        }
        Event::Timer(id) => {
            with(|rt| rt.phase = Phase::BeginTimeStep);
            let w = with(|rt| rt.timers.remove(&id));
            if let Some(w) = w {
                w.wake();
            }
        }
        Event::ReadWrite => {
            with(|rt| rt.phase = Phase::ValuesSettle);
            let (writes, wakers) = with(|rt| {
                rt.rw.cb = None;
                rt.rw.gen += 1;
                let writes = std::mem::take(&mut rt.writes);
                rt.write_index.clear();
                (writes, std::mem::take(&mut rt.rw.waiters))
            });
            // Apply buffered deposits before waking anyone, so writes made
            // earlier in the timestep are visible in the ReadWrite phase.
            for w in writes {
                let r = with(|rt| {
                    let v = owned_as_value(&w.value);
                    rt.backend.write(w.handle, v, Action::Deposit)
                });
                if let Err(e) = r {
                    log::error!("scheduled write failed: {e}");
                }
            }
            for w in wakers {
                w.wake();
            }
        }
        Event::ReadOnly => {
            with(|rt| rt.phase = Phase::EndTimeStep);
            let wakers = with(|rt| {
                rt.ro.cb = None;
                rt.ro.gen += 1;
                std::mem::take(&mut rt.ro.waiters)
            });
            for w in wakers {
                w.wake();
            }
        }
        Event::NextTimeStep => {
            with(|rt| rt.phase = Phase::BeginTimeStep);
            let wakers = with(|rt| {
                rt.nts.cb = None;
                rt.nts.gen += 1;
                std::mem::take(&mut rt.nts.waiters)
            });
            for w in wakers {
                w.wake();
            }
        }
    }
}

fn owned_as_value(v: &OwnedValue) -> Value<'_> {
    match v {
        OwnedValue::Vec(x) => Value::Vec(x),
        OwnedValue::Int(x) => Value::Int(*x),
        OwnedValue::Real(x) => Value::Real(*x),
        OwnedValue::Str(x) => Value::Str(x),
    }
}

// ---------------------------------------------------------------------------
// Trigger registration (called by trigger futures)

impl Runtime {
    fn collect_edge_wakers(&mut self, h: Handle) -> Vec<Waker> {
        let Some(reg) = self.edges.get_mut(&h) else { return Vec::new() };
        if reg.waiters.is_empty() {
            return Vec::new();
        }
        // Determine the new scalar value for edge filtering.
        let mut scratch = std::mem::replace(&mut reg.scratch, LogicVec::zeros(0));
        let bit = match self.backend.read_vec(h, &mut scratch) {
            Ok(()) if scratch.width() >= 1 => Some(scratch.bit(0)),
            _ => None,
        };
        let reg = self.edges.get_mut(&h).unwrap();
        reg.scratch = scratch;
        let mut out = Vec::new();
        reg.waiters.retain(|w| {
            let fire = match w.kind {
                EdgeKind::Any => true,
                EdgeKind::Rising => bit == Some(Logic::One),
                EdgeKind::Falling => bit == Some(Logic::Zero),
            };
            if fire {
                out.push(w.waker.clone());
            }
            !fire
        });
        out
    }

    /// Register an edge waiter; returns the waiter id for deregistration.
    pub(crate) fn add_edge_waiter(&mut self, h: Handle, kind: EdgeKind, waker: Waker) -> u64 {
        let id = self.next_waiter_id;
        self.next_waiter_id += 1;
        if !self.edges.contains_key(&h) {
            let cb = self
                .backend
                .register(CbKind::ValueChange(h))
                .unwrap_or_else(|e| panic!("cannot register value change callback: {e}"));
            self.edges.insert(h, EdgeReg { _cb: cb, waiters: Vec::new(), scratch: LogicVec::zeros(0) });
        }
        self.edges.get_mut(&h).unwrap().waiters.push(EdgeWaiter { id, kind, waker });
        id
    }

    pub(crate) fn remove_edge_waiter(&mut self, h: Handle, id: u64) {
        if let Some(reg) = self.edges.get_mut(&h) {
            reg.waiters.retain(|w| w.id != id);
        }
    }

    pub(crate) fn add_timer(&mut self, steps: u64, waker: Waker) -> CbId {
        let id =
            self.backend.register(CbKind::AfterDelay(steps)).unwrap_or_else(|e| panic!("cannot register timer: {e}"));
        self.timers.insert(id, waker);
        id
    }

    pub(crate) fn remove_timer(&mut self, id: CbId) {
        if self.timers.remove(&id).is_some() {
            let _ = self.backend.remove(id);
        }
    }

    fn phase_waiters(&mut self, kind: CbKind) -> &mut PhaseWaiters {
        match kind {
            CbKind::ReadWrite => &mut self.rw,
            CbKind::ReadOnly => &mut self.ro,
            CbKind::NextTimeStep => &mut self.nts,
            _ => unreachable!(),
        }
    }

    /// Register for the next firing of a phase; returns the generation the
    /// waiter is waiting for.
    pub(crate) fn add_phase_waiter(&mut self, kind: CbKind, waker: Waker) -> u64 {
        let need_cb = self.phase_waiters(kind).cb.is_none();
        if need_cb {
            let id = self.backend.register(kind).unwrap_or_else(|e| panic!("cannot register {kind:?}: {e}"));
            self.phase_waiters(kind).cb = Some(id);
        }
        let pw = self.phase_waiters(kind);
        pw.waiters.push(waker);
        pw.gen
    }

    /// Re-arm a waker on an already-registered phase wait.
    pub(crate) fn add_phase_waiter_existing(&mut self, kind: CbKind, waker: Waker) {
        self.phase_waiters(kind).waiters.push(waker);
    }

    pub(crate) fn phase_fired(&mut self, kind: CbKind, gen: u64) -> bool {
        self.phase_waiters(kind).gen > gen
    }

    pub(crate) fn timers_contains(&self, id: CbId) -> bool {
        self.timers.contains_key(&id)
    }

    pub(crate) fn update_timer_waker(&mut self, id: CbId, waker: Waker) {
        if let Some(w) = self.timers.get_mut(&id) {
            *w = waker;
        }
    }

    /// `true` if the waiter is still registered (updating its waker).
    pub(crate) fn edge_waiter_pending(&mut self, h: Handle, id: u64, waker: &Waker) -> bool {
        if let Some(reg) = self.edges.get_mut(&h) {
            if let Some(w) = reg.waiters.iter_mut().find(|w| w.id == id) {
                if !w.waker.will_wake(waker) {
                    w.waker = waker.clone();
                }
                return true;
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Values

    pub(crate) fn schedule_write(
        &mut self,
        h: Handle,
        value: OwnedValue,
        action: Action,
    ) -> crate::backend::Result<()> {
        if self.phase == Phase::EndTimeStep {
            panic!("writing to a signal in the ReadOnly phase is not allowed");
        }
        let write_now =
            action != Action::Deposit || self.caps.trusts_inertial_writes || self.phase == Phase::ValuesSettle;
        if write_now {
            return self.backend.write(h, owned_as_value(&value), action);
        }
        match self.write_index.get(&h) {
            Some(&i) => {
                // Latest write to a handle wins, but keep first-write order.
                self.writes[i].value = value;
            }
            None => {
                self.write_index.insert(h, self.writes.len());
                self.writes.push(PendingWrite { handle: h, value });
            }
        }
        if self.rw.cb.is_none() {
            self.rw.cb = Some(
                self.backend
                    .register(CbKind::ReadWrite)
                    .unwrap_or_else(|e| panic!("cannot register ReadWrite for write flush: {e}")),
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Test driver support

/// A future that resolves when a failure is recorded against the current
/// test (a task panicked).
pub struct TestFailed;

impl Future for TestFailed {
    type Output = String;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<String> {
        with(|rt| match rt.current_test.as_mut() {
            Some(t) => match t.failure.clone() {
                Some(f) => Poll::Ready(f),
                None => {
                    t.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            },
            None => Poll::Pending,
        })
    }
}

/// Resolves when a failure is recorded against the current test.
pub fn test_failed() -> TestFailed {
    TestFailed
}

/// Mark the start of a test so task panics are attributed to it.
pub fn begin_test() {
    with(|rt| rt.current_test = Some(CurrentTest { failure: None, waker: None }));
}

/// End the current test, returning any recorded failure.
pub fn end_test() -> Option<String> {
    with(|rt| rt.current_test.take().and_then(|t| t.failure))
}

/// Ask the backend to end the simulation.
pub fn finish() {
    with(|rt| {
        rt.finished = true;
        rt.backend.finish();
    });
}

/// Exit code decided by the regression (0 pass, 1 failures).
pub fn exit_code() -> i32 {
    with(|rt| rt.exit_code)
}

pub fn root() -> Option<Handle> {
    with(|rt| rt.root)
}

/// Direct access to the backend for backend crates and tests.
pub fn backend<R>(f: impl FnOnce(&mut dyn Backend) -> R) -> R {
    with(|rt| f(rt.backend.as_mut()))
}

pub fn set_root(h: Handle) {
    with(|rt| rt.root = Some(h));
}

/// Number of live tasks.
pub fn live_tasks() -> usize {
    with(|rt| rt.exec.live_count())
}
