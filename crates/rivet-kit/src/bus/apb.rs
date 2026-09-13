//! APB (AMBA 3/4) master and memory-backed slave.

use super::{full_strobe, opt_sig, sig, Backpressure};
use crate::memory::Memory;
use rivet_core::error::{Error, Result};
use rivet_core::handle::{Module, Signal};
use rivet_core::task::{spawn_named, JoinHandle};

/// The APB signal bundle. `pstrb`, `pprot` and `pslverr` are optional.
#[derive(Clone)]
pub struct Apb {
    pub clk: Signal,
    pub psel: Signal,
    pub penable: Signal,
    pub pwrite: Signal,
    pub paddr: Signal,
    pub pwdata: Signal,
    pub prdata: Signal,
    pub pready: Signal,
    pub pslverr: Option<Signal>,
    pub pstrb: Option<Signal>,
    pub pprot: Option<Signal>,
}

impl Apb {
    pub fn find(m: &Module, clk: Signal, prefix: &str) -> Result<Apb> {
        Ok(Apb {
            clk,
            psel: sig(m, prefix, "psel")?,
            penable: sig(m, prefix, "penable")?,
            pwrite: sig(m, prefix, "pwrite")?,
            paddr: sig(m, prefix, "paddr")?,
            pwdata: sig(m, prefix, "pwdata")?,
            prdata: sig(m, prefix, "prdata")?,
            pready: sig(m, prefix, "pready")?,
            pslverr: opt_sig(m, prefix, "pslverr"),
            pstrb: opt_sig(m, prefix, "pstrb"),
            pprot: opt_sig(m, prefix, "pprot"),
        })
    }

    pub fn data_bytes(&self) -> u32 {
        self.pwdata.width().div_ceil(8)
    }
}

pub struct ApbMaster {
    pub bus: Apb,
}

impl ApbMaster {
    pub fn new(bus: Apb) -> ApbMaster {
        bus.psel.set(0);
        bus.penable.set(0);
        bus.pwrite.set(0);
        if let Some(p) = bus.pprot {
            p.set(0);
        }
        ApbMaster { bus }
    }

    /// Setup phase, then access phase held until `pready`. Returns
    /// `(prdata, pslverr)`.
    async fn transfer(&mut self, write: bool, addr: u64, data: u64, strb: u64) -> (u64, bool) {
        let b = &self.bus;
        b.paddr.set(addr);
        b.pwrite.set(write);
        b.pwdata.set(data);
        if let Some(s) = b.pstrb {
            s.set(if write { strb } else { 0 });
        }
        b.psel.set(1);
        b.penable.set(0);
        b.clk.rising_edge().await;
        b.penable.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.pready.get_u64_lossy() != 0 {
                break;
            }
        }
        let rdata = b.prdata.get_u64_lossy();
        let err = b.pslverr.map(|s| s.get_u64_lossy() != 0).unwrap_or(false);
        b.psel.set(0);
        b.penable.set(0);
        (rdata, err)
    }

    pub async fn write(&mut self, addr: u64, data: u64) -> Result<()> {
        let strb = full_strobe(self.bus.data_bytes());
        let (_, err) = self.transfer(true, addr, data, strb).await;
        if err {
            Err(Error::Msg(format!("APB write to {addr:#x} returned PSLVERR")))
        } else {
            Ok(())
        }
    }

    pub async fn write_strb(&mut self, addr: u64, data: u64, strb: u64) -> bool {
        self.transfer(true, addr, data, strb).await.1
    }

    pub async fn read(&mut self, addr: u64) -> Result<u64> {
        let (v, err) = self.transfer(false, addr, 0, 0).await;
        if err {
            Err(Error::Msg(format!("APB read from {addr:#x} returned PSLVERR")))
        } else {
            Ok(v)
        }
    }
}

/// Memory-backed responder.
pub struct ApbSlave {
    pub bus: Apb,
    pub mem: Memory,
    bp: Backpressure,
    error_ranges: Vec<(u64, u64)>,
}

impl ApbSlave {
    pub fn new(bus: Apb, mem: Memory) -> ApbSlave {
        ApbSlave { bus, mem, bp: Backpressure::None, error_ranges: Vec::new() }
    }

    /// Wait states (cycles with `pready` low in the access phase).
    pub fn backpressure(mut self, bp: Backpressure) -> ApbSlave {
        self.bp = bp;
        self
    }

    pub fn error_range(mut self, lo: u64, hi: u64) -> ApbSlave {
        self.error_ranges.push((lo, hi));
        self
    }

    pub fn run(self) -> JoinHandle<()> {
        let ApbSlave { bus: b, mem, mut bp, error_ranges } = self;
        spawn_named("apb_slave", async move {
            let bytes = b.data_bytes();
            b.pready.set(0);
            b.prdata.set(0);
            if let Some(e) = b.pslverr {
                e.set(0);
            }
            let mut n = 0u64;
            loop {
                // Wait for the access phase.
                loop {
                    b.clk.rising_edge().await;
                    if b.psel.get_u64_lossy() != 0 && b.penable.get_u64_lossy() != 0 {
                        break;
                    }
                }
                let stall = bp.stall(n);
                super::stall_cycles(b.clk, stall).await;
                let addr = b.paddr.get_u64_lossy();
                let err = error_ranges.iter().any(|(lo, hi)| *lo <= addr && addr < *hi);
                if b.pwrite.get_u64_lossy() != 0 {
                    if !err {
                        let strb = b.pstrb.map(|s| s.get_u64_lossy()).unwrap_or(full_strobe(bytes));
                        mem.write_word(addr, bytes, b.pwdata.get_u64_lossy(), strb);
                    }
                } else {
                    b.prdata.set(if err { 0 } else { mem.read_word(addr, bytes) });
                }
                if let Some(e) = b.pslverr {
                    e.set(err);
                }
                b.pready.set(1);
                b.clk.rising_edge().await;
                b.pready.set(0);
                if let Some(e) = b.pslverr {
                    e.set(0);
                }
                n += 1;
            }
        })
    }
}
