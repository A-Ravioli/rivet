//! Bus models: masters (drivers), memory-backed slaves (responders), and
//! streams, for the common on-chip protocols.
//!
//! | protocol | master | slave / responder | signals |
//! |---|---|---|---|
//! | AXI4-Lite | [`AxiLiteMaster`] | [`AxiLiteSlave`] | [`AxiLite`] |
//! | AXI4 | [`AxiMaster`] | [`AxiSlave`] | [`Axi`] |
//! | AXI4-Stream | [`AxisSource`] | [`AxisSink`] | [`Axis`] |
//! | APB | [`ApbMaster`] | [`ApbSlave`] | [`Apb`] |
//! | Avalon-MM | [`AvalonMaster`] | [`AvalonSlave`] | [`Avalon`] |
//! | Wishbone (classic) | [`WishboneMaster`] | [`WishboneSlave`] | [`Wishbone`] |
//!
//! Every signal bundle has a `find(module, clk, prefix)` constructor that
//! looks up `<prefix><signal>` (for example `s_axil_awaddr` with prefix
//! `s_axil_`), with optional signals left `None` when absent.
//!
//! Timing convention shared by all models: outputs are driven right after
//! a rising edge (values-change phase) and inputs are sampled at the next
//! rising edge, so a handshake completes on the edge where both `valid`
//! and `ready` were high during the preceding cycle. Slaves add wait
//! states according to a [`Backpressure`] policy.

pub mod apb;
pub mod avalon;
pub mod axi;
pub mod axi_lite;
pub mod axis;
pub mod wishbone;

pub use apb::{Apb, ApbMaster, ApbSlave};
pub use avalon::{Avalon, AvalonMaster, AvalonSlave};
pub use axi::{Axi, AxiBurst, AxiMaster, AxiSlave};
pub use axi_lite::{AxiLite, AxiLiteMaster, AxiLiteSlave};
pub use axis::{Axis, AxisBeat, AxisSink, AxisSource};
pub use wishbone::{Wishbone, WishboneMaster, WishboneSlave};

use rivet_core::error::{Error, Result};
use rivet_core::handle::{Module, Signal};
use rivet_core::random::Rng;

/// AXI response codes (also used as the generic error indication).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Resp {
    Okay = 0,
    ExOkay = 1,
    SlvErr = 2,
    DecErr = 3,
}

impl Resp {
    pub fn from_bits(v: u64) -> Resp {
        match v & 3 {
            0 => Resp::Okay,
            1 => Resp::ExOkay,
            2 => Resp::SlvErr,
            _ => Resp::DecErr,
        }
    }

    pub fn is_ok(self) -> bool {
        matches!(self, Resp::Okay | Resp::ExOkay)
    }
}

/// Wait-state policy for a responder (or gaps between a driver's
/// transactions): how many cycles to stall before accepting the next item.
pub enum Backpressure {
    /// Accept immediately.
    None,
    /// Always stall `n` cycles.
    Fixed(u32),
    /// Stall a uniform random number of cycles in `min..=max`.
    Random { min: u32, max: u32, rng: Rng },
    /// Decide per item from the item index.
    Custom(Box<dyn FnMut(u64) -> u32>),
}

impl Backpressure {
    /// Random stalls drawn from the test's seeded stream.
    pub fn random(min: u32, max: u32) -> Backpressure {
        Backpressure::Random { min, max, rng: rivet_core::random::rng() }
    }

    /// Cycles to stall before item `index`.
    pub fn stall(&mut self, index: u64) -> u32 {
        match self {
            Backpressure::None => 0,
            Backpressure::Fixed(n) => *n,
            Backpressure::Random { min, max, rng } => rng.gen_range(*min..=*max),
            Backpressure::Custom(f) => f(index),
        }
    }
}

/// `<prefix><name>` in `m`.
pub fn sig(m: &Module, prefix: &str, name: &str) -> Result<Signal> {
    m.signal(&format!("{prefix}{name}"))
        .map_err(|e| Error::Msg(format!("bus signal {prefix}{name} not found in {}: {e}", m.path())))
}

/// `<prefix><name>` if present.
pub fn opt_sig(m: &Module, prefix: &str, name: &str) -> Option<Signal> {
    m.signal(&format!("{prefix}{name}")).ok()
}

/// Full byte strobe for a data width.
pub fn full_strobe(data_bytes: u32) -> u64 {
    if data_bytes >= 64 {
        u64::MAX
    } else {
        (1u64 << data_bytes) - 1
    }
}

/// Wait `n` rising edges (0 returns immediately).
pub async fn stall_cycles(clk: Signal, n: u32) {
    for _ in 0..n {
        clk.rising_edge().await;
    }
}
