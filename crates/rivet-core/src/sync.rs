//! Inter-task synchronisation: `Event`, `Queue`, `Lock`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// A flag tasks can wait on. `set()` wakes every waiter; the flag stays set
/// until `clear()`.
#[derive(Clone, Default)]
pub struct Event {
    inner: Rc<RefCell<EventInner>>,
}

#[derive(Default)]
struct EventInner {
    set: bool,
    waiters: Vec<Waker>,
}

impl Event {
    pub fn new() -> Event {
        Event::default()
    }

    pub fn set(&self) {
        let waiters = {
            let mut g = self.inner.borrow_mut();
            g.set = true;
            std::mem::take(&mut g.waiters)
        };
        for w in waiters {
            w.wake();
        }
    }

    pub fn clear(&self) {
        self.inner.borrow_mut().set = false;
    }

    pub fn is_set(&self) -> bool {
        self.inner.borrow().set
    }

    /// Resolves once the event is set (immediately if it already is).
    pub fn wait(&self) -> EventWait {
        EventWait { inner: self.inner.clone() }
    }
}

pub struct EventWait {
    inner: Rc<RefCell<EventInner>>,
}

impl Future for EventWait {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut g = self.inner.borrow_mut();
        if g.set {
            Poll::Ready(())
        } else {
            g.waiters.push(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// A FIFO channel between tasks. `None` capacity means unbounded.
#[derive(Clone)]
pub struct Queue<T> {
    inner: Rc<RefCell<QueueInner<T>>>,
}

struct QueueInner<T> {
    items: VecDeque<T>,
    capacity: Option<usize>,
    getters: VecDeque<Waker>,
    putters: VecDeque<Waker>,
}

impl<T> Queue<T> {
    pub fn new() -> Queue<T> {
        Queue::with_capacity(None)
    }

    pub fn bounded(capacity: usize) -> Queue<T> {
        Queue::with_capacity(Some(capacity))
    }

    fn with_capacity(capacity: Option<usize>) -> Queue<T> {
        Queue {
            inner: Rc::new(RefCell::new(QueueInner {
                items: VecDeque::new(),
                capacity,
                getters: VecDeque::new(),
                putters: VecDeque::new(),
            })),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.borrow().items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_full(&self) -> bool {
        let g = self.inner.borrow();
        g.capacity.map(|c| g.items.len() >= c).unwrap_or(false)
    }

    /// Put without waiting; `Err(item)` if full.
    pub fn try_put(&self, item: T) -> Result<(), T> {
        let waker = {
            let mut g = self.inner.borrow_mut();
            if g.capacity.map(|c| g.items.len() >= c).unwrap_or(false) {
                return Err(item);
            }
            g.items.push_back(item);
            g.getters.pop_front()
        };
        if let Some(w) = waker {
            w.wake();
        }
        Ok(())
    }

    pub fn try_get(&self) -> Option<T> {
        let (item, waker) = {
            let mut g = self.inner.borrow_mut();
            let item = g.items.pop_front();
            let w = if item.is_some() { g.putters.pop_front() } else { None };
            (item, w)
        };
        if let Some(w) = waker {
            w.wake();
        }
        item
    }

    /// Put, waiting while the queue is full.
    pub async fn put(&self, item: T) {
        let mut item = Some(item);
        loop {
            match self.try_put(item.take().unwrap()) {
                Ok(()) => return,
                Err(back) => {
                    item = Some(back);
                    QueueWait { inner: self.inner.clone(), for_get: false }.await;
                }
            }
        }
    }

    /// Get, waiting while the queue is empty.
    pub async fn get(&self) -> T {
        loop {
            if let Some(v) = self.try_get() {
                return v;
            }
            QueueWait { inner: self.inner.clone(), for_get: true }.await;
        }
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Queue::new()
    }
}

struct QueueWait<T> {
    inner: Rc<RefCell<QueueInner<T>>>,
    for_get: bool,
}

impl<T> Future for QueueWait<T> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut g = self.inner.borrow_mut();
        let ready =
            if self.for_get { !g.items.is_empty() } else { !g.capacity.map(|c| g.items.len() >= c).unwrap_or(false) };
        if ready {
            Poll::Ready(())
        } else {
            if self.for_get {
                g.getters.push_back(cx.waker().clone());
            } else {
                g.putters.push_back(cx.waker().clone());
            }
            Poll::Pending
        }
    }
}

/// An async mutual-exclusion lock. Fair: waiters are woken in order.
#[derive(Clone, Default)]
pub struct Lock {
    inner: Rc<RefCell<LockInner>>,
}

#[derive(Default)]
struct LockInner {
    locked: bool,
    waiters: VecDeque<Waker>,
}

pub struct LockGuard {
    inner: Rc<RefCell<LockInner>>,
}

impl Lock {
    pub fn new() -> Lock {
        Lock::default()
    }

    pub fn is_locked(&self) -> bool {
        self.inner.borrow().locked
    }

    pub async fn acquire(&self) -> LockGuard {
        LockAcquire { inner: self.inner.clone() }.await;
        LockGuard { inner: self.inner.clone() }
    }
}

struct LockAcquire {
    inner: Rc<RefCell<LockInner>>,
}

impl Future for LockAcquire {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut g = self.inner.borrow_mut();
        if !g.locked {
            g.locked = true;
            Poll::Ready(())
        } else {
            g.waiters.push_back(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let w = {
            let mut g = self.inner.borrow_mut();
            g.locked = false;
            g.waiters.pop_front()
        };
        if let Some(w) = w {
            w.wake();
        }
    }
}
