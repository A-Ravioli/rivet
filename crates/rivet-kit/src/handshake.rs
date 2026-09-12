//! Valid/ready handshake driver and monitor.
//!
//! Timing: signals are driven right after a rising edge (values-change
//! phase) and sampled at the next rising edge, where `ready` still holds
//! the value the design drove during the previous cycle. That matches a
//! synchronous design sampling `valid` and `ready` on the same edge.

use crate::{Driver, Monitor};
use rivet_core::handle::Signal;
use rivet_core::triggers::read_only;
use rivet_core::value::{IntoLogicVec, LogicVec};

/// Drives `valid`/`data` and waits for `ready`.
pub struct ValidReadySource {
    pub clk: Signal,
    pub valid: Signal,
    pub ready: Signal,
    pub data: Signal,
}

impl ValidReadySource {
    pub fn new(clk: Signal, valid: Signal, ready: Signal, data: Signal) -> ValidReadySource {
        valid.set(0);
        ValidReadySource { clk, valid, ready, data }
    }

    /// Present `value` and hold it until the design accepts it.
    pub async fn send_value(&mut self, value: impl IntoLogicVec) {
        self.data.set(value);
        self.valid.set(1);
        loop {
            self.clk.rising_edge().await;
            if self.ready.get_u64_lossy() != 0 {
                break;
            }
        }
        self.valid.set(0);
    }

    /// Wait `cycles` idle cycles before the next transaction.
    pub async fn idle(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.clk.rising_edge().await;
        }
    }
}

impl<T: IntoLogicVec> Driver<T> for ValidReadySource {
    async fn send(&mut self, item: T) {
        self.send_value(item).await;
    }
}

/// Observes `valid && ready` transfers, optionally driving `ready` itself.
pub struct ValidReadySink {
    pub clk: Signal,
    pub valid: Signal,
    pub ready: Signal,
    pub data: Signal,
    /// If set, the sink drives `ready`; this closure decides its value
    /// each cycle (return `true` for always-ready).
    pub backpressure: Option<Box<dyn FnMut(u64) -> bool>>,
    cycle: u64,
}

impl ValidReadySink {
    /// A sink that only observes; the design or someone else drives `ready`.
    pub fn observer(clk: Signal, valid: Signal, ready: Signal, data: Signal) -> ValidReadySink {
        ValidReadySink { clk, valid, ready, data, backpressure: None, cycle: 0 }
    }

    /// A sink that drives `ready` according to `policy(cycle)`.
    pub fn with_ready(
        clk: Signal,
        valid: Signal,
        ready: Signal,
        data: Signal,
        policy: impl FnMut(u64) -> bool + 'static,
    ) -> ValidReadySink {
        let mut s = ValidReadySink { clk, valid, ready, data, backpressure: Some(Box::new(policy)), cycle: 0 };
        s.drive_ready();
        s
    }

    fn drive_ready(&mut self) {
        if let Some(p) = self.backpressure.as_mut() {
            let v = p(self.cycle);
            self.ready.set(v);
        }
    }

    /// Wait for the next accepted transfer and return its data, sampled in
    /// the ReadOnly phase of the transfer's edge.
    pub async fn recv_value(&mut self) -> LogicVec {
        loop {
            self.clk.rising_edge().await;
            let fire = self.valid.get_u64_lossy() != 0 && self.ready.get_u64_lossy() != 0;
            self.cycle += 1;
            let data = if fire { Some(self.data.get()) } else { None };
            // Update ready for the coming cycle after sampling this edge.
            self.drive_ready();
            if let Some(d) = data {
                return d;
            }
            read_only().await;
        }
    }
}

impl Monitor<LogicVec> for ValidReadySink {
    async fn recv(&mut self) -> LogicVec {
        self.recv_value().await
    }
}
