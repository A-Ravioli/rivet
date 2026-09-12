//! Avalon-MM master and memory-backed slave (with or without
//! `readdatavalid` pipelining).

use super::{full_strobe, opt_sig, sig, Backpressure};
use crate::memory::Memory;
use rivet_core::error::Result;
use rivet_core::handle::{Module, Signal};
use rivet_core::task::{spawn_named, JoinHandle};

/// The Avalon-MM signal bundle. `byteenable`, `readdatavalid` and
/// `waitrequest` are optional (a missing `waitrequest` means never waiting).
#[derive(Clone)]
pub struct Avalon {
    pub clk: Signal,
    pub address: Signal,
    pub read: Signal,
    pub write: Signal,
    pub readdata: Signal,
    pub writedata: Signal,
    pub byteenable: Option<Signal>,
    pub waitrequest: Option<Signal>,
    pub readdatavalid: Option<Signal>,
}

impl Avalon {
    pub fn find(m: &Module, clk: Signal, prefix: &str) -> Result<Avalon> {
        Ok(Avalon {
            clk,
            address: sig(m, prefix, "address")?,
            read: sig(m, prefix, "read")?,
            write: sig(m, prefix, "write")?,
            readdata: sig(m, prefix, "readdata")?,
            writedata: sig(m, prefix, "writedata")?,
            byteenable: opt_sig(m, prefix, "byteenable"),
            waitrequest: opt_sig(m, prefix, "waitrequest"),
            readdatavalid: opt_sig(m, prefix, "readdatavalid"),
        })
    }

    pub fn data_bytes(&self) -> u32 {
        self.writedata.width().div_ceil(8)
    }
}

pub struct AvalonMaster {
    pub bus: Avalon,
}

impl AvalonMaster {
    pub fn new(bus: Avalon) -> AvalonMaster {
        bus.read.set(0);
        bus.write.set(0);
        if let Some(be) = bus.byteenable {
            be.set(full_strobe(bus.data_bytes()));
        }
        AvalonMaster { bus }
    }

    fn waiting(&self) -> bool {
        self.bus.waitrequest.map(|w| w.get_u64_lossy() != 0).unwrap_or(false)
    }

    pub async fn write(&mut self, addr: u64, data: u64) {
        let be = full_strobe(self.bus.data_bytes());
        self.write_be(addr, data, be).await;
    }

    pub async fn write_be(&mut self, addr: u64, data: u64, byteenable: u64) {
        let b = &self.bus;
        b.address.set(addr);
        b.writedata.set(data);
        if let Some(be) = b.byteenable {
            be.set(byteenable);
        }
        b.write.set(1);
        loop {
            b.clk.rising_edge().await;
            if !self.waiting() {
                break;
            }
        }
        self.bus.write.set(0);
    }

    /// Read: with `readdatavalid`, the command is accepted when
    /// `waitrequest` is low and the data arrives later; without it, the
    /// data is valid in the cycle the command is accepted.
    pub async fn read(&mut self, addr: u64) -> u64 {
        let b = &self.bus;
        b.address.set(addr);
        b.read.set(1);
        loop {
            b.clk.rising_edge().await;
            if !self.waiting() {
                break;
            }
        }
        let b = &self.bus;
        b.read.set(0);
        match b.readdatavalid {
            // Non-pipelined: readdata is valid on the edge that accepted the
            // command (the slave settled it during the previous cycle).
            None => b.readdata.get_u64_lossy(),
            Some(rdv) => {
                // The data may already be valid on the accepting edge.
                if rdv.get_u64_lossy() != 0 {
                    return b.readdata.get_u64_lossy();
                }
                loop {
                    b.clk.rising_edge().await;
                    if rdv.get_u64_lossy() != 0 {
                        return b.readdata.get_u64_lossy();
                    }
                }
            }
        }
    }
}

/// Memory-backed responder. Drives `waitrequest` per the policy and, when
/// present, `readdatavalid` one cycle after accepting a read.
pub struct AvalonSlave {
    pub bus: Avalon,
    pub mem: Memory,
    bp: Backpressure,
}

impl AvalonSlave {
    pub fn new(bus: Avalon, mem: Memory) -> AvalonSlave {
        AvalonSlave { bus, mem, bp: Backpressure::None }
    }

    pub fn backpressure(mut self, bp: Backpressure) -> AvalonSlave {
        self.bp = bp;
        self
    }

    pub fn run(self) -> JoinHandle<()> {
        let AvalonSlave { bus: b, mem, mut bp } = self;
        spawn_named("avalon_slave", async move {
            let bytes = b.data_bytes();
            if let Some(w) = b.waitrequest {
                w.set(1);
            }
            if let Some(v) = b.readdatavalid {
                v.set(0);
            }
            b.readdata.set(0);
            let mut n = 0u64;
            loop {
                // Wait for a command, then hold waitrequest for the stall.
                loop {
                    b.clk.rising_edge().await;
                    if let Some(v) = b.readdatavalid {
                        v.set(0);
                    }
                    if b.read.get_u64_lossy() != 0 || b.write.get_u64_lossy() != 0 {
                        break;
                    }
                }
                let stall = bp.stall(n);
                super::stall_cycles(b.clk, stall).await;
                let addr = b.address.get_u64_lossy();
                let be = b.byteenable.map(|s| s.get_u64_lossy()).unwrap_or(full_strobe(bytes));
                if b.write.get_u64_lossy() != 0 {
                    mem.write_word(addr, bytes, b.writedata.get_u64_lossy(), be);
                } else {
                    b.readdata.set(mem.read_word(addr, bytes));
                }
                if let Some(w) = b.waitrequest {
                    w.set(0);
                }
                // The master samples waitrequest low on this edge: command
                // accepted; readdata is valid here for non-pipelined reads.
                b.clk.rising_edge().await;
                if let Some(w) = b.waitrequest {
                    w.set(1);
                }
                if let Some(v) = b.readdatavalid {
                    v.set(1);
                }
                n += 1;
            }
        })
    }
}
