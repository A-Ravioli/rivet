//! AXI4-Lite master and memory-backed slave.

use super::{full_strobe, opt_sig, sig, Backpressure, Resp};
use crate::memory::Memory;
use rivet_core::error::{Error, Result};
use rivet_core::handle::{Module, Signal};
use rivet_core::task::{spawn_named, JoinHandle};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// The AXI4-Lite signal bundle.
#[derive(Clone)]
pub struct AxiLite {
    pub clk: Signal,
    pub awaddr: Signal,
    pub awprot: Option<Signal>,
    pub awvalid: Signal,
    pub awready: Signal,
    pub wdata: Signal,
    pub wstrb: Signal,
    pub wvalid: Signal,
    pub wready: Signal,
    pub bresp: Signal,
    pub bvalid: Signal,
    pub bready: Signal,
    pub araddr: Signal,
    pub arprot: Option<Signal>,
    pub arvalid: Signal,
    pub arready: Signal,
    pub rdata: Signal,
    pub rresp: Signal,
    pub rvalid: Signal,
    pub rready: Signal,
}

impl AxiLite {
    pub fn find(m: &Module, clk: Signal, prefix: &str) -> Result<AxiLite> {
        Ok(AxiLite {
            clk,
            awaddr: sig(m, prefix, "awaddr")?,
            awprot: opt_sig(m, prefix, "awprot"),
            awvalid: sig(m, prefix, "awvalid")?,
            awready: sig(m, prefix, "awready")?,
            wdata: sig(m, prefix, "wdata")?,
            wstrb: sig(m, prefix, "wstrb")?,
            wvalid: sig(m, prefix, "wvalid")?,
            wready: sig(m, prefix, "wready")?,
            bresp: sig(m, prefix, "bresp")?,
            bvalid: sig(m, prefix, "bvalid")?,
            bready: sig(m, prefix, "bready")?,
            araddr: sig(m, prefix, "araddr")?,
            arprot: opt_sig(m, prefix, "arprot"),
            arvalid: sig(m, prefix, "arvalid")?,
            arready: sig(m, prefix, "arready")?,
            rdata: sig(m, prefix, "rdata")?,
            rresp: sig(m, prefix, "rresp")?,
            rvalid: sig(m, prefix, "rvalid")?,
            rready: sig(m, prefix, "rready")?,
        })
    }

    pub fn data_bytes(&self) -> u32 {
        self.wdata.width().div_ceil(8)
    }
}

/// Drives the master side: one transaction at a time.
pub struct AxiLiteMaster {
    pub bus: AxiLite,
}

impl AxiLiteMaster {
    /// Take the bus and drive it idle.
    pub fn new(bus: AxiLite) -> AxiLiteMaster {
        bus.awvalid.set(0);
        bus.wvalid.set(0);
        bus.bready.set(0);
        bus.arvalid.set(0);
        bus.rready.set(0);
        if let Some(p) = bus.awprot {
            p.set(0);
        }
        if let Some(p) = bus.arprot {
            p.set(0);
        }
        AxiLiteMaster { bus }
    }

    /// Write `data` with a full strobe; `Err` on a non-OKAY response.
    pub async fn write(&mut self, addr: u64, data: u64) -> Result<()> {
        let strb = full_strobe(self.bus.data_bytes());
        let resp = self.write_strb(addr, data, strb).await;
        if resp.is_ok() {
            Ok(())
        } else {
            Err(Error::Msg(format!("AXI-Lite write to {addr:#x} returned {resp:?}")))
        }
    }

    /// Write with an explicit byte strobe and return the response.
    pub async fn write_strb(&mut self, addr: u64, data: u64, strb: u64) -> Resp {
        let b = &self.bus;
        b.awaddr.set(addr);
        b.awvalid.set(1);
        b.wdata.set(data);
        b.wstrb.set(strb);
        b.wvalid.set(1);
        let (mut aw_done, mut w_done) = (false, false);
        while !(aw_done && w_done) {
            b.clk.rising_edge().await;
            if !aw_done && b.awready.get_u64_lossy() != 0 {
                aw_done = true;
                b.awvalid.set(0);
            }
            if !w_done && b.wready.get_u64_lossy() != 0 {
                w_done = true;
                b.wvalid.set(0);
            }
        }
        b.bready.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.bvalid.get_u64_lossy() != 0 {
                break;
            }
        }
        let resp = Resp::from_bits(b.bresp.get_u64_lossy());
        b.bready.set(0);
        resp
    }

    /// Read; `Err` on a non-OKAY response.
    pub async fn read(&mut self, addr: u64) -> Result<u64> {
        let (v, resp) = self.read_resp(addr).await;
        if resp.is_ok() {
            Ok(v)
        } else {
            Err(Error::Msg(format!("AXI-Lite read from {addr:#x} returned {resp:?}")))
        }
    }

    /// Read and return the data with its response.
    pub async fn read_resp(&mut self, addr: u64) -> (u64, Resp) {
        let b = &self.bus;
        b.araddr.set(addr);
        b.arvalid.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.arready.get_u64_lossy() != 0 {
                break;
            }
        }
        b.arvalid.set(0);
        b.rready.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.rvalid.get_u64_lossy() != 0 {
                break;
            }
        }
        let v = b.rdata.get_u64_lossy();
        let resp = Resp::from_bits(b.rresp.get_u64_lossy());
        b.rready.set(0);
        (v, resp)
    }
}

/// Memory-backed responder. Addresses in an error range answer SLVERR.
pub struct AxiLiteSlave {
    pub bus: AxiLite,
    pub mem: Memory,
    write_bp: Backpressure,
    read_bp: Backpressure,
    error_ranges: Vec<(u64, u64)>,
}

impl AxiLiteSlave {
    pub fn new(bus: AxiLite, mem: Memory) -> AxiLiteSlave {
        AxiLiteSlave { bus, mem, write_bp: Backpressure::None, read_bp: Backpressure::None, error_ranges: Vec::new() }
    }

    /// Wait states before accepting each write (address and data) and
    /// before each read address.
    pub fn backpressure(mut self, write: Backpressure, read: Backpressure) -> AxiLiteSlave {
        self.write_bp = write;
        self.read_bp = read;
        self
    }

    /// Accesses in `lo..hi` answer SLVERR and do not touch memory.
    pub fn error_range(mut self, lo: u64, hi: u64) -> AxiLiteSlave {
        self.error_ranges.push((lo, hi));
        self
    }

    /// Start the responder tasks (write channels and read channels).
    pub fn run(self) -> (JoinHandle<()>, JoinHandle<()>) {
        let AxiLiteSlave { bus, mem, write_bp, read_bp, error_ranges } = self;
        let errs = Rc::new(error_ranges);
        let bytes = bus.data_bytes();
        let is_err = {
            let errs = errs.clone();
            move |a: u64| errs.iter().any(|(lo, hi)| *lo <= a && a < *hi)
        };
        let is_err2 = is_err.clone();
        let b = bus.clone();
        let m = mem.clone();
        let write_task = spawn_named("axil_slave_write", async move {
            let mut bp = write_bp;
            b.awready.set(0);
            b.wready.set(0);
            b.bvalid.set(0);
            b.bresp.set(0);
            let mut n = 0u64;
            loop {
                // Accept the address and the data, in either order, each
                // after its own stall.
                let stall = bp.stall(n);
                super::stall_cycles(b.clk, stall).await;
                b.awready.set(1);
                b.wready.set(1);
                let (mut addr, mut data, mut strb) = (None, None, 0u64);
                while addr.is_none() || data.is_none() {
                    b.clk.rising_edge().await;
                    if addr.is_none() && b.awvalid.get_u64_lossy() != 0 {
                        addr = Some(b.awaddr.get_u64_lossy());
                        b.awready.set(0);
                    }
                    if data.is_none() && b.wvalid.get_u64_lossy() != 0 {
                        data = Some(b.wdata.get_u64_lossy());
                        strb = b.wstrb.get_u64_lossy();
                        b.wready.set(0);
                    }
                }
                let (addr, data) = (addr.unwrap(), data.unwrap());
                let resp = if is_err(addr) {
                    Resp::SlvErr
                } else {
                    m.write_word(addr, bytes, data, strb);
                    Resp::Okay
                };
                b.bresp.set(resp as u64);
                b.bvalid.set(1);
                loop {
                    b.clk.rising_edge().await;
                    if b.bready.get_u64_lossy() != 0 {
                        break;
                    }
                }
                b.bvalid.set(0);
                n += 1;
            }
        });
        let b = bus;
        let read_task = spawn_named("axil_slave_read", async move {
            let mut bp = read_bp;
            b.arready.set(0);
            b.rvalid.set(0);
            b.rresp.set(0);
            b.rdata.set(0);
            let mut n = 0u64;
            loop {
                let stall = bp.stall(n);
                super::stall_cycles(b.clk, stall).await;
                b.arready.set(1);
                let addr = loop {
                    b.clk.rising_edge().await;
                    if b.arvalid.get_u64_lossy() != 0 {
                        break b.araddr.get_u64_lossy();
                    }
                };
                b.arready.set(0);
                let (data, resp) =
                    if is_err2(addr) { (0, Resp::SlvErr) } else { (mem.read_word(addr, bytes), Resp::Okay) };
                b.rdata.set(data);
                b.rresp.set(resp as u64);
                b.rvalid.set(1);
                loop {
                    b.clk.rising_edge().await;
                    if b.rready.get_u64_lossy() != 0 {
                        break;
                    }
                }
                b.rvalid.set(0);
                n += 1;
            }
        });
        (write_task, read_task)
    }
}

/// A queue of completed writes observed by a passive monitor.
#[derive(Clone, Default)]
pub struct AxiLiteLog {
    pub writes: Rc<RefCell<VecDeque<(u64, u64, u64)>>>,
    pub reads: Rc<RefCell<VecDeque<(u64, u64)>>>,
}

/// Passive monitor: records `(addr, data, strb)` for writes and
/// `(addr, data)` for reads as they complete on the bus.
pub fn axi_lite_monitor(bus: AxiLite, log: AxiLiteLog) -> JoinHandle<()> {
    spawn_named("axil_monitor", async move {
        let mut aw: VecDeque<u64> = VecDeque::new();
        let mut w: VecDeque<(u64, u64)> = VecDeque::new();
        let mut ar: VecDeque<u64> = VecDeque::new();
        loop {
            bus.clk.rising_edge().await;
            if bus.awvalid.get_u64_lossy() != 0 && bus.awready.get_u64_lossy() != 0 {
                aw.push_back(bus.awaddr.get_u64_lossy());
            }
            if bus.wvalid.get_u64_lossy() != 0 && bus.wready.get_u64_lossy() != 0 {
                w.push_back((bus.wdata.get_u64_lossy(), bus.wstrb.get_u64_lossy()));
            }
            while !aw.is_empty() && !w.is_empty() {
                let a = aw.pop_front().unwrap();
                let (d, s) = w.pop_front().unwrap();
                log.writes.borrow_mut().push_back((a, d, s));
            }
            if bus.arvalid.get_u64_lossy() != 0 && bus.arready.get_u64_lossy() != 0 {
                ar.push_back(bus.araddr.get_u64_lossy());
            }
            if bus.rvalid.get_u64_lossy() != 0 && bus.rready.get_u64_lossy() != 0 {
                if let Some(a) = ar.pop_front() {
                    log.reads.borrow_mut().push_back((a, bus.rdata.get_u64_lossy()));
                }
            }
        }
    })
}
