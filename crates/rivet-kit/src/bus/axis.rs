//! AXI4-Stream source and sink with packets, `tkeep`, `tlast`, `tuser`,
//! and backpressure.

use super::{opt_sig, sig, Backpressure};
use rivet_core::error::Result;
use rivet_core::handle::{Module, Signal};
use rivet_core::triggers::read_only;

/// The AXI4-Stream signal bundle. Only `tvalid`, `tready` and `tdata` are
/// required.
#[derive(Clone)]
pub struct Axis {
    pub clk: Signal,
    pub tvalid: Signal,
    pub tready: Signal,
    pub tdata: Signal,
    pub tlast: Option<Signal>,
    pub tkeep: Option<Signal>,
    pub tstrb: Option<Signal>,
    pub tuser: Option<Signal>,
    pub tid: Option<Signal>,
    pub tdest: Option<Signal>,
}

impl Axis {
    pub fn find(m: &Module, clk: Signal, prefix: &str) -> Result<Axis> {
        Ok(Axis {
            clk,
            tvalid: sig(m, prefix, "tvalid")?,
            tready: sig(m, prefix, "tready")?,
            tdata: sig(m, prefix, "tdata")?,
            tlast: opt_sig(m, prefix, "tlast"),
            tkeep: opt_sig(m, prefix, "tkeep"),
            tstrb: opt_sig(m, prefix, "tstrb"),
            tuser: opt_sig(m, prefix, "tuser"),
            tid: opt_sig(m, prefix, "tid"),
            tdest: opt_sig(m, prefix, "tdest"),
        })
    }

    pub fn data_bytes(&self) -> u32 {
        self.tdata.width().div_ceil(8)
    }
}

/// One transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AxisBeat {
    pub data: u64,
    pub keep: u64,
    pub last: bool,
    pub user: u64,
}

impl AxisBeat {
    pub fn new(data: u64) -> AxisBeat {
        AxisBeat { data, keep: u64::MAX, last: false, user: 0 }
    }
}

/// Drives beats and packets; gaps between beats follow a [`Backpressure`].
pub struct AxisSource {
    pub bus: Axis,
    gaps: Backpressure,
    sent: u64,
}

impl AxisSource {
    pub fn new(bus: Axis) -> AxisSource {
        bus.tvalid.set(0);
        for s in [bus.tlast, bus.tkeep, bus.tstrb, bus.tuser, bus.tid, bus.tdest].into_iter().flatten() {
            s.set(0);
        }
        AxisSource { bus, gaps: Backpressure::None, sent: 0 }
    }

    /// Idle cycles inserted before each beat.
    pub fn gaps(mut self, gaps: Backpressure) -> AxisSource {
        self.gaps = gaps;
        self
    }

    /// Present one beat until it is accepted.
    pub async fn send_beat(&mut self, beat: &AxisBeat) {
        let b = &self.bus;
        let gap = self.gaps.stall(self.sent);
        super::stall_cycles(b.clk, gap).await;
        b.tdata.set(beat.data);
        if let Some(k) = b.tkeep {
            b.tkeep.unwrap().set(beat.keep & super::full_strobe(k.width()));
        }
        if let Some(s) = b.tstrb {
            s.set(beat.keep & super::full_strobe(s.width()));
        }
        if let Some(l) = b.tlast {
            l.set(beat.last);
        }
        if let Some(u) = b.tuser {
            u.set(beat.user);
        }
        b.tvalid.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.tready.get_u64_lossy() != 0 {
                break;
            }
        }
        b.tvalid.set(0);
        if let Some(l) = b.tlast {
            l.set(0);
        }
        self.sent += 1;
    }

    /// Send bytes as a packet: `data_bytes` per beat, little-endian lanes,
    /// `tkeep` marking the valid bytes of the final beat, `tlast` on it.
    pub async fn send_packet(&mut self, bytes: &[u8]) {
        let n = self.bus.data_bytes() as usize;
        assert!(n <= 8, "send_packet supports up to 64-bit data; use send_beat for wider buses");
        let chunks: Vec<&[u8]> = bytes.chunks(n).collect();
        for (i, c) in chunks.iter().enumerate() {
            let mut data = 0u64;
            for (j, byte) in c.iter().enumerate() {
                data |= (*byte as u64) << (8 * j);
            }
            let keep = (1u64 << c.len()) - 1;
            self.send_beat(&AxisBeat { data, keep, last: i + 1 == chunks.len(), user: 0 }).await;
        }
    }
}

/// Receives beats and packets, driving `tready` from a [`Backpressure`].
pub struct AxisSink {
    pub bus: Axis,
    ready_bp: Backpressure,
    received: u64,
}

impl AxisSink {
    pub fn new(bus: Axis) -> AxisSink {
        bus.tready.set(0);
        AxisSink { bus, ready_bp: Backpressure::None, received: 0 }
    }

    /// Cycles to hold `tready` low before accepting each beat.
    pub fn backpressure(mut self, bp: Backpressure) -> AxisSink {
        self.ready_bp = bp;
        self
    }

    /// Wait for the next accepted beat.
    pub async fn recv_beat(&mut self) -> AxisBeat {
        let b = &self.bus;
        let stall = self.ready_bp.stall(self.received);
        super::stall_cycles(b.clk, stall).await;
        b.tready.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.tvalid.get_u64_lossy() != 0 {
                break;
            }
        }
        let beat = AxisBeat {
            data: b.tdata.get_u64_lossy(),
            keep: b.tkeep.map(|k| k.get_u64_lossy()).unwrap_or(u64::MAX),
            last: b.tlast.map(|l| l.get_u64_lossy() != 0).unwrap_or(false),
            user: b.tuser.map(|u| u.get_u64_lossy()).unwrap_or(0),
        };
        b.tready.set(0);
        self.received += 1;
        beat
    }

    /// Collect beats until `tlast`, returning the kept bytes.
    pub async fn recv_packet(&mut self) -> Vec<u8> {
        let n = self.bus.data_bytes();
        let mut out = Vec::new();
        loop {
            let beat = self.recv_beat().await;
            for j in 0..n.min(8) {
                if beat.keep >> j & 1 == 1 {
                    out.push((beat.data >> (8 * j)) as u8);
                }
            }
            if beat.last || self.bus.tlast.is_none() {
                break;
            }
        }
        out
    }
}

/// Count accepted beats on a stream without driving anything (for
/// throughput checks). Runs until cancelled; the count is in `counter`.
pub async fn axis_count_transfers(bus: Axis, counter: std::rc::Rc<std::cell::Cell<u64>>) {
    loop {
        bus.clk.rising_edge().await;
        read_only().await;
        if bus.tvalid.get_u64_lossy() != 0 && bus.tready.get_u64_lossy() != 0 {
            counter.set(counter.get() + 1);
        }
    }
}
