//! Verification components built on `rivet-core`.
//!
//! - [`reset`]: drive a reset for some cycles.
//! - [`Scoreboard`]: compare observed transactions with expected ones.
//! - [`ValidReadySource`] / [`ValidReadySink`]: a valid/ready handshake
//!   driver and monitor.
//! - [`Driver`] / [`Monitor`]: traits for user components, with helpers to
//!   run them as tasks fed by a [`Queue`].

pub mod handshake;
pub mod scoreboard;

pub use handshake::{ValidReadySink, ValidReadySource};
pub use scoreboard::Scoreboard;

use rivet_core::handle::Signal;
use rivet_core::sync::Queue;
use rivet_core::task::{spawn_named, JoinHandle};
use std::future::Future;

/// Hold `rst` asserted for `cycles` rising edges of `clk`, then release it
/// and wait one more edge. Returns in the values-change phase of that edge.
pub async fn reset(clk: Signal, rst: Signal, active_low: bool, cycles: u32) {
    rst.set(!active_low);
    for _ in 0..cycles {
        clk.rising_edge().await;
    }
    rst.set(active_low);
    clk.rising_edge().await;
}

/// Something that can send transactions into the design.
pub trait Driver<T> {
    fn send(&mut self, item: T) -> impl Future<Output = ()>;
}

/// Something that observes transactions from the design.
pub trait Monitor<T> {
    fn recv(&mut self) -> impl Future<Output = T>;
}

/// Run a driver as a task that drains `queue` until cancelled.
pub fn drive_from_queue<T: 'static, D: Driver<T> + 'static>(
    name: &str,
    mut driver: D,
    queue: Queue<T>,
) -> JoinHandle<()> {
    spawn_named(name, async move {
        loop {
            let item = queue.get().await;
            driver.send(item).await;
        }
    })
}

/// Run a monitor as a task that pushes into `queue` until cancelled.
pub fn monitor_to_queue<T: 'static, M: Monitor<T> + 'static>(
    name: &str,
    mut monitor: M,
    queue: Queue<T>,
) -> JoinHandle<()> {
    spawn_named(name, async move {
        loop {
            let item = monitor.recv().await;
            queue.put(item).await;
        }
    })
}
