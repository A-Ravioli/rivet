//! Verification components built on `rivet-core`.
//!
//! - [`reset`]: drive a reset for some cycles.
//! - [`Scoreboard`]: compare observed transactions with expected ones.
//! - [`ValidReadySource`] / [`ValidReadySink`]: a valid/ready handshake
//!   driver and monitor.
//! - [`Driver`] / [`Monitor`]: traits for user components, with helpers to
//!   run them as tasks fed by a [`Queue`].
//! - [`Memory`]: sparse byte memory with `$readmemh` loading.
//! - [`bus`]: AXI4-Lite, AXI4, AXI4-Stream, APB, Avalon-MM and Wishbone
//!   masters, memory-backed slaves and streams.
//! - [`Reset`]: synchronous and asynchronous reset sequences.
//! - [`Model`] / [`ModelScoreboard`]: reference models feeding a scoreboard.
//! - [`check`]: assertion-style checkers (`assert_stable`, `assert_never`,
//!   `assert_implies`, `assert_no_x`, ...).
//! - [`Trace`]: transaction traces compared with golden files.

pub mod bus;
pub mod check;
pub mod handshake;
pub mod memory;
pub mod model;
#[cfg(feature = "python")]
pub mod python;
pub mod reset;
pub mod scoreboard;
pub mod trace;

pub use bus::Backpressure;
pub use check::{
    assert_always, assert_becomes, assert_implies, assert_never, assert_no_x, assert_stable, assert_within,
};
pub use handshake::{ValidReadySink, ValidReadySource};
pub use memory::{load_hex_into, Memory};
pub use model::{Model, ModelScoreboard};
#[cfg(feature = "python")]
pub use python::PyModel;
pub use reset::Reset;
pub use scoreboard::Scoreboard;
pub use trace::Trace;

use rivet_core::handle::Signal;
use rivet_core::sync::Queue;
use rivet_core::task::{spawn_named, JoinHandle};
use std::future::Future;

/// Hold `rst` asserted for `cycles` rising edges of `clk`, then release it
/// and wait one more edge. Returns in the values-change phase of that edge.
/// See [`Reset`] for asynchronous resets and other options.
pub async fn reset(clk: Signal, rst: Signal, active_low: bool, cycles: u32) {
    let r = Reset::new(clk, rst).cycles(cycles).settle(1);
    let r = if active_low { r.active_low() } else { r };
    r.apply().await
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
