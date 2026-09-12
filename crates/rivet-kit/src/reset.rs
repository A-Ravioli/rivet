//! Reset sequencing.
//!
//! ```ignore
//! Reset::new(clk, rst_n).active_low().cycles(3).apply().await;          // synchronous
//! Reset::new(clk, arst).asynchronous().cycles(2).settle(2).apply().await;
//! ```

use rivet_core::handle::Signal;

/// A reset sequence. Defaults: active high, synchronous, asserted for two
/// rising edges, one settling edge after release.
pub struct Reset {
    clk: Signal,
    rst: Signal,
    active_low: bool,
    asynchronous: bool,
    cycles: u32,
    settle: u32,
}

impl Reset {
    pub fn new(clk: Signal, rst: Signal) -> Reset {
        Reset { clk, rst, active_low: false, asynchronous: false, cycles: 2, settle: 1 }
    }

    /// The reset is asserted when the signal is 0.
    pub fn active_low(mut self) -> Reset {
        self.active_low = true;
        self
    }

    pub fn active_high(mut self) -> Reset {
        self.active_low = false;
        self
    }

    /// Asynchronous reset: released half a cycle after the last asserted
    /// rising edge (on a falling edge), so release never races the flops'
    /// sampling edge.
    pub fn asynchronous(mut self) -> Reset {
        self.asynchronous = true;
        self
    }

    /// Synchronous reset: asserted and released just after rising edges,
    /// so the design samples the change on the following edge.
    pub fn synchronous(mut self) -> Reset {
        self.asynchronous = false;
        self
    }

    /// Rising edges the reset stays asserted for.
    pub fn cycles(mut self, n: u32) -> Reset {
        self.cycles = n;
        self
    }

    /// Rising edges to wait after release before returning.
    pub fn settle(mut self, n: u32) -> Reset {
        self.settle = n;
        self
    }

    /// Run the sequence. Returns just after a rising edge (values-change
    /// phase) with the reset released.
    pub async fn apply(self) {
        let asserted = !self.active_low;
        let released = self.active_low;
        self.rst.set(asserted);
        for _ in 0..self.cycles.max(1) {
            self.clk.rising_edge().await;
        }
        if self.asynchronous {
            self.clk.falling_edge().await;
        }
        self.rst.set(released);
        for _ in 0..self.settle.max(1) {
            self.clk.rising_edge().await;
        }
    }
}
