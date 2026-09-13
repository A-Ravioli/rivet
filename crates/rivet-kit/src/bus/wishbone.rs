//! Wishbone B4 classic (single-cycle handshake) master and memory-backed
//! slave.

use super::{full_strobe, opt_sig, sig, Backpressure};
use crate::memory::Memory;
use rivet_core::error::{Error, Result};
use rivet_core::handle::{Module, Signal};
use rivet_core::task::{spawn_named, JoinHandle};

/// The Wishbone signal bundle, named from the master's point of view:
/// `dat_w` is the master's write data, `dat_r` the slave's read data.
/// `sel`, `err` and `rty` are optional.
#[derive(Clone)]
pub struct Wishbone {
    pub clk: Signal,
    pub cyc: Signal,
    pub stb: Signal,
    pub we: Signal,
    pub adr: Signal,
    pub dat_w: Signal,
    pub dat_r: Signal,
    pub ack: Signal,
    pub sel: Option<Signal>,
    pub err: Option<Signal>,
    pub rty: Option<Signal>,
}

impl Wishbone {
    pub fn find(m: &Module, clk: Signal, prefix: &str) -> Result<Wishbone> {
        Ok(Wishbone {
            clk,
            cyc: sig(m, prefix, "cyc")?,
            stb: sig(m, prefix, "stb")?,
            we: sig(m, prefix, "we")?,
            adr: sig(m, prefix, "adr")?,
            dat_w: sig(m, prefix, "dat_w")?,
            dat_r: sig(m, prefix, "dat_r")?,
            ack: sig(m, prefix, "ack")?,
            sel: opt_sig(m, prefix, "sel"),
            err: opt_sig(m, prefix, "err"),
            rty: opt_sig(m, prefix, "rty"),
        })
    }

    pub fn data_bytes(&self) -> u32 {
        self.dat_w.width().div_ceil(8)
    }
}

pub struct WishboneMaster {
    pub bus: Wishbone,
}

impl WishboneMaster {
    pub fn new(bus: Wishbone) -> WishboneMaster {
        bus.cyc.set(0);
        bus.stb.set(0);
        bus.we.set(0);
        if let Some(s) = bus.sel {
            s.set(full_strobe(bus.data_bytes()));
        }
        WishboneMaster { bus }
    }

    /// One classic cycle; `Ok(dat_r)` on `ack`, `Err` on `err`.
    async fn cycle(&mut self, we: bool, addr: u64, data: u64, sel: u64) -> Result<u64> {
        let b = &self.bus;
        b.adr.set(addr);
        b.we.set(we);
        b.dat_w.set(data);
        if let Some(s) = b.sel {
            s.set(sel);
        }
        b.cyc.set(1);
        b.stb.set(1);
        let r = loop {
            b.clk.rising_edge().await;
            if b.ack.get_u64_lossy() != 0 {
                break Ok(b.dat_r.get_u64_lossy());
            }
            if b.err.map(|e| e.get_u64_lossy() != 0).unwrap_or(false) {
                break Err(Error::Msg(format!(
                    "Wishbone {} at {addr:#x} returned ERR",
                    if we { "write" } else { "read" }
                )));
            }
        };
        b.cyc.set(0);
        b.stb.set(0);
        r
    }

    pub async fn write(&mut self, addr: u64, data: u64) -> Result<()> {
        let sel = full_strobe(self.bus.data_bytes());
        self.cycle(true, addr, data, sel).await.map(|_| ())
    }

    pub async fn write_sel(&mut self, addr: u64, data: u64, sel: u64) -> Result<()> {
        self.cycle(true, addr, data, sel).await.map(|_| ())
    }

    pub async fn read(&mut self, addr: u64) -> Result<u64> {
        let sel = full_strobe(self.bus.data_bytes());
        self.cycle(false, addr, 0, sel).await
    }
}

/// Memory-backed responder.
pub struct WishboneSlave {
    pub bus: Wishbone,
    pub mem: Memory,
    bp: Backpressure,
    error_ranges: Vec<(u64, u64)>,
}

impl WishboneSlave {
    pub fn new(bus: Wishbone, mem: Memory) -> WishboneSlave {
        WishboneSlave { bus, mem, bp: Backpressure::None, error_ranges: Vec::new() }
    }

    /// Wait states before `ack`.
    pub fn backpressure(mut self, bp: Backpressure) -> WishboneSlave {
        self.bp = bp;
        self
    }

    /// Accesses in `lo..hi` answer `err` (if the bus has it) instead of `ack`.
    pub fn error_range(mut self, lo: u64, hi: u64) -> WishboneSlave {
        self.error_ranges.push((lo, hi));
        self
    }

    pub fn run(self) -> JoinHandle<()> {
        let WishboneSlave { bus: b, mem, mut bp, error_ranges } = self;
        spawn_named("wishbone_slave", async move {
            let bytes = b.data_bytes();
            b.ack.set(0);
            b.dat_r.set(0);
            if let Some(e) = b.err {
                e.set(0);
            }
            if let Some(r) = b.rty {
                r.set(0);
            }
            let mut n = 0u64;
            loop {
                loop {
                    b.clk.rising_edge().await;
                    if b.cyc.get_u64_lossy() != 0 && b.stb.get_u64_lossy() != 0 {
                        break;
                    }
                }
                let stall = bp.stall(n);
                super::stall_cycles(b.clk, stall).await;
                let addr = b.adr.get_u64_lossy();
                let is_err = error_ranges.iter().any(|(lo, hi)| *lo <= addr && addr < *hi) && b.err.is_some();
                if !is_err {
                    if b.we.get_u64_lossy() != 0 {
                        let sel = b.sel.map(|s| s.get_u64_lossy()).unwrap_or(full_strobe(bytes));
                        mem.write_word(addr, bytes, b.dat_w.get_u64_lossy(), sel);
                    } else {
                        b.dat_r.set(mem.read_word(addr, bytes));
                    }
                    b.ack.set(1);
                } else {
                    b.err.unwrap().set(1);
                }
                b.clk.rising_edge().await;
                b.ack.set(0);
                if let Some(e) = b.err {
                    e.set(0);
                }
                n += 1;
            }
        })
    }
}
