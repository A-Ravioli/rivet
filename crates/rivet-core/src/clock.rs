//! Clock generation.

use crate::backend::Action;
use crate::handle::Signal;
use crate::task::{spawn_named, JoinHandle};
use crate::time::Duration;
use crate::triggers::Timer;
use crate::{runtime, value::LogicVec};

/// A running clock on a signal. Dropping the `Clock` does not stop it; call
/// [`Clock::stop`].
pub struct Clock {
    signal: Signal,
    period_steps: u64,
    task: JoinHandle<()>,
}

/// Configuration for a clock.
pub struct ClockBuilder {
    signal: Signal,
    period: Duration,
    high_steps: Option<u64>,
    start_high: bool,
    action: Action,
}

impl ClockBuilder {
    /// Half-period high time (default: period / 2).
    pub fn high_time(mut self, d: Duration) -> Self {
        self.high_steps = Some(d.to_steps(runtime::precision()));
        self
    }

    /// Whether the first edge is a rising edge (default `true`).
    pub fn start_high(mut self, v: bool) -> Self {
        self.start_high = v;
        self
    }

    /// Drive the clock with immediate (`NoDelay`) writes instead of
    /// inertial deposits. Deposits are the default so that a value written
    /// before an edge in the same time step is flushed before the edge, as
    /// in cocotb; immediate writes are cheaper on simulators that do not
    /// honour inertial writes.
    pub fn immediate(mut self) -> Self {
        self.action = Action::NoDelay;
        self
    }

    pub fn start(self) -> Clock {
        let precision = runtime::precision();
        let period = self.period.to_steps(precision);
        assert!(period >= 2, "clock period must be at least two simulator steps");
        let high = self.high_steps.unwrap_or(period / 2);
        assert!(high > 0 && high < period, "clock high time must be within the period");
        let low = period - high;
        let signal = self.signal;
        let action = self.action;
        let start_high = self.start_high;
        let one = LogicVec::from_u64(1, 1);
        let zero = LogicVec::from_u64(1, 0);
        let task = spawn_named(&format!("clock({})", signal.path()), async move {
            let mut high_next = start_high;
            loop {
                let v = if high_next { &one } else { &zero };
                runtime::with(|rt| rt.schedule_write(signal.handle(), crate::backend::OwnedValue::Vec(v.clone()), action))
                    .unwrap_or_else(|e| panic!("clock write failed: {e}"));
                Timer::steps(if high_next { high } else { low }).await;
                high_next = !high_next;
            }
        });
        Clock { signal, period_steps: period, task }
    }
}

impl Clock {
    /// Configure a clock with the given period.
    pub fn new(signal: Signal, period: Duration) -> ClockBuilder {
        ClockBuilder { signal, period, high_steps: None, start_high: true, action: Action::Deposit }
    }

    /// Start a 50% duty-cycle clock, first edge rising.
    pub fn start(signal: Signal, period: Duration) -> Clock {
        Clock::new(signal, period).start()
    }

    pub fn stop(&self) {
        self.task.cancel();
    }

    pub fn signal(&self) -> Signal {
        self.signal
    }

    pub fn period_steps(&self) -> u64 {
        self.period_steps
    }

    /// Wait for `n` rising edges.
    pub async fn cycles(&self, n: u32) {
        for _ in 0..n {
            self.signal.rising_edge().await;
        }
    }
}
