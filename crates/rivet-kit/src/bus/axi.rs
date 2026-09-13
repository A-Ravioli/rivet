//! AXI4 (full) master and memory-backed slave: bursts (FIXED, INCR, WRAP),
//! narrow transfers, IDs echoed, one transaction in flight per direction.

use super::{opt_sig, sig, Backpressure, Resp};
use crate::memory::Memory;
use rivet_core::error::{Error, Result};
use rivet_core::handle::{Module, Signal};
use rivet_core::task::{spawn_named, JoinHandle};

/// Burst type encodings.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AxiBurst {
    Fixed = 0,
    Incr = 1,
    Wrap = 2,
}

impl AxiBurst {
    pub fn from_bits(v: u64) -> AxiBurst {
        match v & 3 {
            0 => AxiBurst::Fixed,
            2 => AxiBurst::Wrap,
            _ => AxiBurst::Incr,
        }
    }
}

/// The AXI4 signal bundle. `awid`/`bid`/`arid`/`rid`, `wlast`/`rlast` and
/// the `x_prot`/`x_lock`/`x_cache`/`x_qos` sidebands are optional.
#[derive(Clone)]
pub struct Axi {
    pub clk: Signal,
    pub awid: Option<Signal>,
    pub awaddr: Signal,
    pub awlen: Signal,
    pub awsize: Signal,
    pub awburst: Signal,
    pub awvalid: Signal,
    pub awready: Signal,
    pub wdata: Signal,
    pub wstrb: Signal,
    pub wlast: Signal,
    pub wvalid: Signal,
    pub wready: Signal,
    pub bid: Option<Signal>,
    pub bresp: Signal,
    pub bvalid: Signal,
    pub bready: Signal,
    pub arid: Option<Signal>,
    pub araddr: Signal,
    pub arlen: Signal,
    pub arsize: Signal,
    pub arburst: Signal,
    pub arvalid: Signal,
    pub arready: Signal,
    pub rid: Option<Signal>,
    pub rdata: Signal,
    pub rresp: Signal,
    pub rlast: Signal,
    pub rvalid: Signal,
    pub rready: Signal,
    /// Sidebands driven to zero by the master if present.
    pub zeros: Vec<Signal>,
}

impl Axi {
    pub fn find(m: &Module, clk: Signal, prefix: &str) -> Result<Axi> {
        let zeros =
            ["awprot", "awlock", "awcache", "awqos", "awregion", "arprot", "arlock", "arcache", "arqos", "arregion"]
                .iter()
                .filter_map(|n| opt_sig(m, prefix, n))
                .collect();
        Ok(Axi {
            clk,
            awid: opt_sig(m, prefix, "awid"),
            awaddr: sig(m, prefix, "awaddr")?,
            awlen: sig(m, prefix, "awlen")?,
            awsize: sig(m, prefix, "awsize")?,
            awburst: sig(m, prefix, "awburst")?,
            awvalid: sig(m, prefix, "awvalid")?,
            awready: sig(m, prefix, "awready")?,
            wdata: sig(m, prefix, "wdata")?,
            wstrb: sig(m, prefix, "wstrb")?,
            wlast: sig(m, prefix, "wlast")?,
            wvalid: sig(m, prefix, "wvalid")?,
            wready: sig(m, prefix, "wready")?,
            bid: opt_sig(m, prefix, "bid"),
            bresp: sig(m, prefix, "bresp")?,
            bvalid: sig(m, prefix, "bvalid")?,
            bready: sig(m, prefix, "bready")?,
            arid: opt_sig(m, prefix, "arid"),
            araddr: sig(m, prefix, "araddr")?,
            arlen: sig(m, prefix, "arlen")?,
            arsize: sig(m, prefix, "arsize")?,
            arburst: sig(m, prefix, "arburst")?,
            arvalid: sig(m, prefix, "arvalid")?,
            arready: sig(m, prefix, "arready")?,
            rid: opt_sig(m, prefix, "rid"),
            rdata: sig(m, prefix, "rdata")?,
            rresp: sig(m, prefix, "rresp")?,
            rlast: sig(m, prefix, "rlast")?,
            rvalid: sig(m, prefix, "rvalid")?,
            rready: sig(m, prefix, "rready")?,
            zeros,
        })
    }

    pub fn data_bytes(&self) -> u32 {
        self.wdata.width().div_ceil(8)
    }
}

/// Address of beat `i` of a burst, per the AXI address computation.
pub fn beat_address(start: u64, size: u32, len: u32, burst: AxiBurst, i: u32) -> u64 {
    let bytes = 1u64 << size;
    let aligned = start & !(bytes - 1);
    match burst {
        AxiBurst::Fixed => start,
        AxiBurst::Incr => {
            if i == 0 {
                start
            } else {
                aligned + bytes * i as u64
            }
        }
        AxiBurst::Wrap => {
            let total = bytes * (len as u64 + 1);
            let lower = start & !(total - 1);
            let upper = lower + total;
            let mut a = aligned + bytes * i as u64;
            if a >= upper {
                a -= total;
            }
            a
        }
    }
}

/// Byte strobe for a beat that only covers the bytes from `addr` to the
/// end of its `size` window within the bus width.
fn narrow_strobe(addr: u64, size: u32, data_bytes: u32) -> u64 {
    let bytes = 1u64 << size;
    let lane = addr % data_bytes as u64;
    let start_in_window = addr % bytes;
    let mut s = 0u64;
    for b in start_in_window..bytes {
        let l = lane - start_in_window + b;
        if l < data_bytes as u64 {
            s |= 1 << l;
        }
    }
    s
}

/// Drives the master side.
pub struct AxiMaster {
    pub bus: Axi,
    pub id: u64,
}

impl AxiMaster {
    pub fn new(bus: Axi) -> AxiMaster {
        for s in [bus.awvalid, bus.wvalid, bus.bready, bus.arvalid, bus.rready] {
            s.set(0);
        }
        for z in &bus.zeros {
            z.set(0);
        }
        AxiMaster { bus, id: 0 }
    }

    /// Write one beat per element of `data`; `size` is log2(bytes per beat).
    /// Data for beat `i` is placed on the lanes its address selects.
    pub async fn write_burst(&mut self, addr: u64, data: &[u64], size: u32, burst: AxiBurst) -> Result<Resp> {
        let b = &self.bus;
        let len = data.len() as u32 - 1;
        assert!(!data.is_empty() && data.len() <= 256, "AXI burst length 1..=256");
        let dbytes = b.data_bytes();
        if let Some(id) = b.awid {
            id.set(self.id);
        }
        b.awaddr.set(addr);
        b.awlen.set(len as u64);
        b.awsize.set(size as u64);
        b.awburst.set(burst as u64);
        b.awvalid.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.awready.get_u64_lossy() != 0 {
                break;
            }
        }
        b.awvalid.set(0);
        for (i, d) in data.iter().enumerate() {
            let a = beat_address(addr, size, len, burst, i as u32);
            let lane = (a % dbytes as u64) as u32;
            let shift = 8 * (lane - (a % (1 << size)) as u32);
            b.wdata.set(d << shift);
            b.wstrb.set(narrow_strobe(a, size, dbytes));
            b.wlast.set(i + 1 == data.len());
            b.wvalid.set(1);
            loop {
                b.clk.rising_edge().await;
                if b.wready.get_u64_lossy() != 0 {
                    break;
                }
            }
        }
        b.wvalid.set(0);
        b.wlast.set(0);
        b.bready.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.bvalid.get_u64_lossy() != 0 {
                break;
            }
        }
        let resp = Resp::from_bits(b.bresp.get_u64_lossy());
        if let Some(bid) = b.bid {
            let got = bid.get_u64_lossy();
            if got != self.id {
                b.bready.set(0);
                return Err(Error::Msg(format!("AXI BID {got} does not match AWID {}", self.id)));
            }
        }
        b.bready.set(0);
        Ok(resp)
    }

    /// Read `len + 1` beats; returns the data of each beat, shifted down to
    /// bit 0.
    pub async fn read_burst(&mut self, addr: u64, len: u32, size: u32, burst: AxiBurst) -> Result<(Vec<u64>, Resp)> {
        let b = &self.bus;
        let dbytes = b.data_bytes();
        if let Some(id) = b.arid {
            id.set(self.id);
        }
        b.araddr.set(addr);
        b.arlen.set(len as u64);
        b.arsize.set(size as u64);
        b.arburst.set(burst as u64);
        b.arvalid.set(1);
        loop {
            b.clk.rising_edge().await;
            if b.arready.get_u64_lossy() != 0 {
                break;
            }
        }
        b.arvalid.set(0);
        b.rready.set(1);
        let mut out = Vec::with_capacity(len as usize + 1);
        let mut resp = Resp::Okay;
        let mut i = 0u32;
        loop {
            b.clk.rising_edge().await;
            if b.rvalid.get_u64_lossy() == 0 {
                continue;
            }
            let a = beat_address(addr, size, len, burst, i);
            let lane = (a % dbytes as u64) as u32;
            let shift = 8 * (lane - (a % (1 << size)) as u32);
            let mask = if size >= 3 { u64::MAX } else { (1u64 << (8 << size)) - 1 };
            out.push((b.rdata.get_u64_lossy() >> shift) & mask);
            let r = Resp::from_bits(b.rresp.get_u64_lossy());
            if !r.is_ok() {
                resp = r;
            }
            if let Some(rid) = b.rid {
                let got = rid.get_u64_lossy();
                if got != self.id {
                    b.rready.set(0);
                    return Err(Error::Msg(format!("AXI RID {got} does not match ARID {}", self.id)));
                }
            }
            let last = b.rlast.get_u64_lossy() != 0;
            i += 1;
            if last || i > len {
                if !last {
                    b.rready.set(0);
                    return Err(Error::Msg(format!("AXI read burst of {} beats did not end with RLAST", len + 1)));
                }
                break;
            }
        }
        b.rready.set(0);
        Ok((out, resp))
    }

    /// Single-beat write of `data_bytes` bytes.
    pub async fn write(&mut self, addr: u64, data: u64) -> Result<()> {
        let size = self.bus.data_bytes().trailing_zeros();
        let r = self.write_burst(addr, &[data], size, AxiBurst::Incr).await?;
        if r.is_ok() {
            Ok(())
        } else {
            Err(Error::Msg(format!("AXI write to {addr:#x} returned {r:?}")))
        }
    }

    /// Single-beat read of `data_bytes` bytes.
    pub async fn read(&mut self, addr: u64) -> Result<u64> {
        let size = self.bus.data_bytes().trailing_zeros();
        let (v, r) = self.read_burst(addr, 0, size, AxiBurst::Incr).await?;
        if r.is_ok() {
            Ok(v[0])
        } else {
            Err(Error::Msg(format!("AXI read from {addr:#x} returned {r:?}")))
        }
    }
}

/// Memory-backed responder.
pub struct AxiSlave {
    pub bus: Axi,
    pub mem: Memory,
    write_bp: Backpressure,
    read_bp: Backpressure,
    error_ranges: Vec<(u64, u64)>,
}

impl AxiSlave {
    pub fn new(bus: Axi, mem: Memory) -> AxiSlave {
        AxiSlave { bus, mem, write_bp: Backpressure::None, read_bp: Backpressure::None, error_ranges: Vec::new() }
    }

    /// Wait states before each write beat and before each read beat.
    pub fn backpressure(mut self, write: Backpressure, read: Backpressure) -> AxiSlave {
        self.write_bp = write;
        self.read_bp = read;
        self
    }

    /// Bursts starting in `lo..hi` answer SLVERR and do not touch memory.
    pub fn error_range(mut self, lo: u64, hi: u64) -> AxiSlave {
        self.error_ranges.push((lo, hi));
        self
    }

    pub fn run(self) -> (JoinHandle<()>, JoinHandle<()>) {
        let AxiSlave { bus, mem, write_bp, read_bp, error_ranges } = self;
        let errs = std::rc::Rc::new(error_ranges);
        let dbytes = bus.data_bytes();
        let b = bus.clone();
        let m = mem.clone();
        let e = errs.clone();
        let write_task = spawn_named("axi_slave_write", async move {
            let mut bp = write_bp;
            for s in [b.awready, b.wready, b.bvalid] {
                s.set(0);
            }
            let mut n = 0u64;
            loop {
                b.awready.set(1);
                let (addr, len, size, burst, id) = loop {
                    b.clk.rising_edge().await;
                    if b.awvalid.get_u64_lossy() != 0 {
                        break (
                            b.awaddr.get_u64_lossy(),
                            b.awlen.get_u64_lossy() as u32,
                            b.awsize.get_u64_lossy() as u32,
                            AxiBurst::from_bits(b.awburst.get_u64_lossy()),
                            b.awid.map(|s| s.get_u64_lossy()).unwrap_or(0),
                        );
                    }
                };
                b.awready.set(0);
                let err = e.iter().any(|(lo, hi)| *lo <= addr && addr < *hi);
                let mut i = 0u32;
                loop {
                    let stall = bp.stall(n);
                    super::stall_cycles(b.clk, stall).await;
                    b.wready.set(1);
                    let (data, strb, last) = loop {
                        b.clk.rising_edge().await;
                        if b.wvalid.get_u64_lossy() != 0 {
                            break (b.wdata.get_u64_lossy(), b.wstrb.get_u64_lossy(), b.wlast.get_u64_lossy() != 0);
                        }
                    };
                    b.wready.set(0);
                    if !err {
                        let a = beat_address(addr, size, len, burst, i);
                        let base = a & !(dbytes as u64 - 1);
                        m.write_word(base, dbytes, data, strb);
                    }
                    n += 1;
                    i += 1;
                    if last || i > len {
                        break;
                    }
                }
                if let Some(bid) = b.bid {
                    bid.set(id);
                }
                b.bresp.set(if err { Resp::SlvErr } else { Resp::Okay } as u64);
                b.bvalid.set(1);
                loop {
                    b.clk.rising_edge().await;
                    if b.bready.get_u64_lossy() != 0 {
                        break;
                    }
                }
                b.bvalid.set(0);
            }
        });
        let b = bus;
        let read_task = spawn_named("axi_slave_read", async move {
            let mut bp = read_bp;
            b.arready.set(0);
            b.rvalid.set(0);
            b.rlast.set(0);
            let mut n = 0u64;
            loop {
                b.arready.set(1);
                let (addr, len, size, burst, id) = loop {
                    b.clk.rising_edge().await;
                    if b.arvalid.get_u64_lossy() != 0 {
                        break (
                            b.araddr.get_u64_lossy(),
                            b.arlen.get_u64_lossy() as u32,
                            b.arsize.get_u64_lossy() as u32,
                            AxiBurst::from_bits(b.arburst.get_u64_lossy()),
                            b.arid.map(|s| s.get_u64_lossy()).unwrap_or(0),
                        );
                    }
                };
                b.arready.set(0);
                let err = errs.iter().any(|(lo, hi)| *lo <= addr && addr < *hi);
                for i in 0..=len {
                    let stall = bp.stall(n);
                    super::stall_cycles(b.clk, stall).await;
                    let a = beat_address(addr, size, len, burst, i);
                    let base = a & !(dbytes as u64 - 1);
                    let data = if err { 0 } else { mem.read_word(base, dbytes) };
                    b.rdata.set(data);
                    b.rresp.set(if err { Resp::SlvErr } else { Resp::Okay } as u64);
                    b.rlast.set(i == len);
                    if let Some(rid) = b.rid {
                        rid.set(id);
                    }
                    b.rvalid.set(1);
                    loop {
                        b.clk.rising_edge().await;
                        if b.rready.get_u64_lossy() != 0 {
                            break;
                        }
                    }
                    b.rvalid.set(0);
                    b.rlast.set(0);
                    n += 1;
                }
            }
        });
        (write_task, read_task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_addresses() {
        assert_eq!(beat_address(0x10, 2, 3, AxiBurst::Incr, 0), 0x10);
        assert_eq!(beat_address(0x10, 2, 3, AxiBurst::Incr, 2), 0x18);
        assert_eq!(beat_address(0x13, 2, 3, AxiBurst::Incr, 1), 0x14, "unaligned start aligns after beat 0");
        assert_eq!(beat_address(0x40, 2, 3, AxiBurst::Fixed, 3), 0x40);
        // 4-beat wrap of 4-byte transfers starting at 0x18: 18, 1c, 10, 14.
        let w: Vec<u64> = (0..4).map(|i| beat_address(0x18, 2, 3, AxiBurst::Wrap, i)).collect();
        assert_eq!(w, [0x18, 0x1c, 0x10, 0x14]);
        assert_eq!(narrow_strobe(0x0, 2, 8), 0x0f);
        assert_eq!(narrow_strobe(0x4, 2, 8), 0xf0);
        assert_eq!(narrow_strobe(0x5, 0, 8), 0x20);
        assert_eq!(narrow_strobe(0x1, 2, 4), 0b1110, "first beat of an unaligned burst");
    }
}
