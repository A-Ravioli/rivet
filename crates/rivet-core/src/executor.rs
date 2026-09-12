//! A single-threaded, deterministic task executor.
//!
//! Tasks are `!Send` futures stored in slots. Wakers push slot keys onto a
//! FIFO ready queue. The executor never owns the thread: the runtime drains
//! the ready queue from inside each simulator callback and returns to the
//! simulator when every task is parked ([`crate::runtime::run_to_idle`]).
//!
//! Polling happens with no borrow of the runtime held, so tasks can freely
//! call back into the runtime (spawn, register triggers, read and write
//! signals).

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};

/// Identifies a task slot plus a generation so stale wakes are ignored.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TaskKey {
    idx: u32,
    gen: u32,
}

pub(crate) struct Task {
    pub fut: Pin<Box<dyn Future<Output = ()>>>,
    pub name: String,
    pub waker: Waker,
    pub queued: Arc<AtomicBool>,
    /// What the task registered for the last time it was polled; shown by
    /// [`crate::runtime::dump_tasks`].
    pub wait: WaitOn,
}

/// What a parked task is waiting for. Recorded by the trigger futures when
/// they register, so a hang can be explained without a debugger.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum WaitOn {
    #[default]
    Unknown,
    /// A timer due at this absolute step.
    Timer(u64),
    Edge(crate::backend::Handle, crate::runtime::EdgeKind),
    ReadWrite,
    ReadOnly,
    NextTimeStep,
    Event,
    Queue,
    Lock,
    Join,
    Yield,
    Other(&'static str),
}

enum SlotState {
    Empty,
    Parked(Task),
    /// Currently being polled by `run_to_idle`.
    Running,
    /// Cancelled while running; dropped when the poll returns.
    Cancelled,
}

struct Slot {
    gen: u32,
    state: SlotState,
}

#[derive(Default)]
pub(crate) struct ReadyQueue {
    queue: Mutex<VecDeque<TaskKey>>,
}

struct TaskWaker {
    key: TaskKey,
    queue: Arc<ReadyQueue>,
    queued: Arc<AtomicBool>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            self.queue.queue.lock().unwrap().push_back(self.key);
        }
    }
}

pub(crate) struct Executor {
    slots: Vec<Slot>,
    free: Vec<u32>,
    ready: Arc<ReadyQueue>,
    pub current: Option<TaskKey>,
    /// Name of the task being polled (for dumps while running).
    pub current_name: String,
    /// Wait recorded by the task currently being polled.
    pub current_wait: WaitOn,
    live: usize,
}

impl Executor {
    pub fn new() -> Executor {
        Executor {
            slots: Vec::new(),
            free: Vec::new(),
            ready: Arc::default(),
            current: None,
            current_name: String::new(),
            current_wait: WaitOn::Unknown,
            live: 0,
        }
    }

    /// `(name, wait)` for every parked task, in slot order.
    pub fn parked(&self) -> Vec<(String, WaitOn)> {
        self.slots
            .iter()
            .filter_map(|s| match &s.state {
                SlotState::Parked(t) => Some((t.name.clone(), t.wait)),
                _ => None,
            })
            .collect()
    }

    pub fn spawn(&mut self, name: String, fut: Pin<Box<dyn Future<Output = ()>>>) -> TaskKey {
        let idx = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(Slot { gen: 0, state: SlotState::Empty });
                (self.slots.len() - 1) as u32
            }
        };
        let slot = &mut self.slots[idx as usize];
        let key = TaskKey { idx, gen: slot.gen };
        let queued = Arc::new(AtomicBool::new(false));
        let waker = Waker::from(Arc::new(TaskWaker { key, queue: self.ready.clone(), queued: queued.clone() }));
        slot.state = SlotState::Parked(Task { fut, name, waker: waker.clone(), queued, wait: WaitOn::Unknown });
        self.live += 1;
        waker.wake();
        key
    }

    /// Pop the next runnable task, marking its slot as running.
    pub fn next_ready(&mut self) -> Option<(TaskKey, Task)> {
        loop {
            let key = self.ready.queue.lock().unwrap().pop_front()?;
            let slot = &mut self.slots[key.idx as usize];
            if slot.gen != key.gen {
                continue;
            }
            match std::mem::replace(&mut slot.state, SlotState::Running) {
                SlotState::Parked(task) => {
                    task.queued.store(false, Ordering::Release);
                    return Some((key, task));
                }
                other => {
                    // Not parked: put the state back and skip.
                    slot.state = other;
                    continue;
                }
            }
        }
    }

    /// Return a task after a `Pending` poll. Returns the task back to the
    /// caller if it was cancelled meanwhile, so it can be dropped without a
    /// runtime borrow held.
    pub fn park(&mut self, key: TaskKey, task: Task) -> Option<Task> {
        let slot = &mut self.slots[key.idx as usize];
        if slot.gen != key.gen {
            return Some(task);
        }
        match slot.state {
            SlotState::Running => {
                slot.state = SlotState::Parked(task);
                None
            }
            SlotState::Cancelled => {
                self.release(key);
                Some(task)
            }
            _ => Some(task),
        }
    }

    /// Free a slot after the task completed.
    pub fn release(&mut self, key: TaskKey) {
        let slot = &mut self.slots[key.idx as usize];
        if slot.gen != key.gen {
            return;
        }
        if !matches!(slot.state, SlotState::Empty) {
            self.live -= 1;
        }
        slot.state = SlotState::Empty;
        slot.gen = slot.gen.wrapping_add(1);
        self.free.push(key.idx);
    }

    /// Cancel a task. Returns the task so the caller drops it outside any
    /// runtime borrow. A task cancelling itself is dropped after its poll.
    pub fn cancel(&mut self, key: TaskKey) -> Option<Task> {
        let slot = self.slots.get_mut(key.idx as usize)?;
        if slot.gen != key.gen {
            return None;
        }
        match std::mem::replace(&mut slot.state, SlotState::Empty) {
            SlotState::Parked(task) => {
                self.live -= 1;
                slot.gen = slot.gen.wrapping_add(1);
                self.free.push(key.idx);
                Some(task)
            }
            SlotState::Running => {
                slot.state = SlotState::Cancelled;
                None
            }
            other => {
                slot.state = other;
                None
            }
        }
    }

    /// Cancel every task except `keep`. Returns the tasks for dropping.
    pub fn cancel_all_except(&mut self, keep: Option<TaskKey>) -> Vec<Task> {
        let mut out = Vec::new();
        for idx in 0..self.slots.len() as u32 {
            let key = TaskKey { idx, gen: self.slots[idx as usize].gen };
            if Some(key) == keep {
                continue;
            }
            if let Some(t) = self.cancel(key) {
                out.push(t);
            }
        }
        out
    }

    pub fn is_alive(&self, key: TaskKey) -> bool {
        self.slots
            .get(key.idx as usize)
            .map(|s| s.gen == key.gen && !matches!(s.state, SlotState::Empty))
            .unwrap_or(false)
    }

    pub fn live_count(&self) -> usize {
        self.live
    }
}
