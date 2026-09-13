//! Bus example: every kit component against real RTL, plus random
//! stimulus, functional coverage, golden traces, checkers, parameter sets
//! and per-value test parametrisation.

mod dut;

use dut::{Cmd, Dut, Op};

use rivet::kit::bus::axi::AxiBurst;
use rivet::kit::bus::{
    Apb, ApbMaster, Avalon, AvalonMaster, Axi, AxiLite, AxiLiteMaster, AxiLiteSlave, AxiMaster, Axis, AxisSink,
    AxisSource, Resp, Wishbone, WishboneMaster,
};
use rivet::kit::{
    assert_implies, assert_never, assert_no_x, assert_stable, Backpressure, Memory, ModelScoreboard, Reset, Trace,
};
use rivet::prelude::*;

async fn reset(dut: &Module) -> rivet::Result<Signal> {
    let clk = dut.signal("clk")?;
    let _ = Clock::start(clk, 10.ns());
    Reset::new(clk, dut.signal("rst_n")?).active_low().cycles(3).settle(1).apply().await;
    Ok(clk)
}

/// A random AXI-Lite transaction; the constraint keeps addresses inside
/// the register file most of the time.
#[derive(Randomize, Debug, Clone)]
#[rand(constraint = |t: &Self| t.addr.is_multiple_of(4))]
struct AxilTxn {
    #[rand(with = |r: &mut Rng| r.gen_range(0..16u64) * 4)]
    addr: u64,
    data: u32,
    #[rand(weighted = [(0xfu64, 6), (0x3, 1), (0xc, 1), (0x1, 1), (0x0, 1)])]
    strb: u64,
    #[rand(weighted = [(true, 1), (false, 1)])]
    write: bool,
}

/// Random transactions against the AXI-Lite register file, checked against
/// a byte memory model, with functional coverage on address and strobe.
#[rivet::test(timeout = 200.us())]
async fn axil_regfile_random(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let bus = AxiLite::find(&dut, clk, "s_axil_")?;
    let mut m = AxiLiteMaster::new(bus.clone());
    // Registers reset to the REG_INIT parameter (see [design.param_sets]).
    let model = Memory::new();
    let init = dut.signal("REG_INIT")?.get_u64()? as u32;
    for i in 0..8 {
        model.write_u32(i * 4, init);
    }
    let cg = Covergroup::new("axil");
    let addr_pt = cg.point("addr", Bins::new().auto("reg", 0..8u64).bin("out_of_range", 8..=15u64));
    let strb_pt = cg.point(
        "strb",
        Bins::new()
            .values("full", [0xfu64])
            .values("half", [0x3u64, 0xc])
            .values("byte", [0x1u64])
            .values("none", [0x0u64]),
    );
    let kind_pt = cg.point("kind", Bins::new().values("read", [0u64]).values("write", [1u64]));
    let _cross = cg.cross("kind_x_addr", &[&kind_pt, &addr_pt]);
    let _no_x = assert_no_x("axil_outputs", clk, vec![bus.bvalid, bus.rvalid, bus.awready, bus.wready]);
    let mut rng = rivet::rng();
    let mut errors = 0;
    for _ in 0..400 {
        let t = AxilTxn::randomize(&mut rng);
        kind_pt.sample(t.write as u64);
        addr_pt.sample(t.addr / 4);
        if t.write {
            strb_pt.sample(t.strb);
            let resp = m.write_strb(t.addr, t.data as u64, t.strb).await;
            if t.addr < 0x20 {
                ensure!(resp == Resp::Okay, "write {:#x} -> {resp:?}", t.addr);
                model.write_word(t.addr, 4, t.data as u64, t.strb);
            } else {
                ensure!(resp == Resp::SlvErr, "write {:#x} should be SLVERR, got {resp:?}", t.addr);
                errors += 1;
            }
        } else {
            let (v, resp) = m.read_resp(t.addr).await;
            if t.addr < 0x20 {
                ensure!(resp == Resp::Okay, "read {:#x} -> {resp:?}", t.addr);
                assert_eq!(v, model.read_u32(t.addr) as u64, "read {:#x}", t.addr);
            } else {
                ensure!(resp == Resp::SlvErr && v == 0, "read {:#x} should be SLVERR", t.addr);
                errors += 1;
            }
        }
    }
    info!("{errors} error responses, coverage {:.1}%", cg.percent());
    ensure!(cg.percent() > 95.0, "coverage {:.1}% too low for 400 transactions", cg.percent());
    Ok(())
}

/// A fixed-seed sequence recorded as a trace and compared with the golden
/// file in `golden/`.
#[rivet::test]
async fn axil_golden_trace(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let bus = AxiLite::find(&dut, clk, "s_axil_")?;
    let mut m = AxiLiteMaster::new(bus);
    let mut trace = Trace::new("axil");
    let mut rng = Rng::seed_from_u64(0x5eed);
    for i in 0..12u64 {
        let addr = rng.gen_range(0..8u64) * 4;
        if i % 3 == 2 {
            let v = m.read(addr).await?;
            trace.record(format!("R {addr:#04x} -> {v:#010x}"));
        } else {
            let data: u32 = rng.gen();
            m.write(addr, data as u64).await?;
            trace.record(format!("W {addr:#04x} <- {data:#010x}"));
        }
    }
    rivet::kit::assert_trace!(trace);
    Ok(())
}

/// Kit master through the DUT's register slice into the kit slave.
#[rivet::test]
async fn axil_slice_to_kit_slave(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let p = AxiLite::find(&dut, clk, "p_axil_")?;
    let q = AxiLite::find(&dut, clk, "q_axil_")?;
    let mem = Memory::new();
    let _slave = AxiLiteSlave::new(q, mem.clone())
        .backpressure(Backpressure::random(0, 3), Backpressure::random(0, 3))
        .error_range(0x8000, 0x9000)
        .run();
    let mut m = AxiLiteMaster::new(p);
    for i in 0..16u64 {
        m.write(0x100 + i * 4, 0xa5a5_0000 + i).await?;
    }
    for i in 0..16u64 {
        assert_eq!(m.read(0x100 + i * 4).await?, 0xa5a5_0000 + i);
        assert_eq!(mem.read_u32(0x100 + i * 4) as u64, 0xa5a5_0000 + i);
    }
    assert_eq!(m.write_strb(0x8000, 1, 0xf).await, Resp::SlvErr);
    Ok(())
}

/// AXI4 bursts against the memory, checked against a `Memory` model; the
/// burst length is a test parameter.
#[rivet::test(params = [1u32, 4, 16], timeout = 500.us())]
async fn axi_mem_bursts(dut: Module, beats: u32) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let bus = Axi::find(&dut, clk, "s_axi_")?;
    let mut m = AxiMaster::new(bus);
    let model = Memory::new();
    let mut rng = rivet::rng();
    for burst in [AxiBurst::Incr, AxiBurst::Wrap, AxiBurst::Fixed] {
        if burst == AxiBurst::Wrap && !matches!(beats, 2 | 4 | 8 | 16) {
            continue;
        }
        for _ in 0..4 {
            let max_start = 1024 - 4 * beats as u64;
            let addr = rng.gen_range(0..=max_start / 4) * 4;
            let addr = if burst == AxiBurst::Wrap {
                addr & !(4 * beats as u64 - 1) | (rng.gen_range(0..beats as u64) * 4)
            } else {
                addr
            };
            let data: Vec<u64> = (0..beats).map(|_| rng.gen::<u32>() as u64).collect();
            m.id = rng.gen_range(0..16u64);
            let resp = m.write_burst(addr, &data, 2, burst).await?;
            ensure!(resp == Resp::Okay, "{burst:?} write at {addr:#x}: {resp:?}");
            for (i, d) in data.iter().enumerate() {
                let a = rivet::kit::bus::axi::beat_address(addr, 2, beats - 1, burst, i as u32);
                model.write_u32(a, *d as u32);
            }
            let (rd, resp) = m.read_burst(addr, beats - 1, 2, burst).await?;
            ensure!(resp == Resp::Okay, "{burst:?} read at {addr:#x}: {resp:?}");
            for (i, v) in rd.iter().enumerate() {
                let a = rivet::kit::bus::axi::beat_address(addr, 2, beats - 1, burst, i as u32);
                assert_eq!(*v, model.read_u32(a) as u64, "{burst:?} beat {i} at {a:#x}");
            }
        }
    }
    // Out of range: SLVERR, memory untouched.
    let r = m.write_burst(0x1000, &[1], 2, AxiBurst::Incr).await?;
    assert_eq!(r, Resp::SlvErr);
    Ok(())
}

/// Packets through the skid buffer with random gaps and backpressure,
/// checked by a model scoreboard, with coverage on packet length.
#[rivet::test(timeout = 500.us())]
async fn axis_skid_packets(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let s = Axis::find(&dut, clk, "s_axis_")?;
    let mo = Axis::find(&dut, clk, "m_axis_")?;
    let mut src = AxisSource::new(s.clone()).gaps(Backpressure::random(0, 2));
    let mut sink = AxisSink::new(mo.clone()).backpressure(Backpressure::random(0, 3));
    let cg = Covergroup::new("axis");
    let len_pt = cg.point("len", Bins::new().bin("one", 1..=1u64).bin("short", 2..=4u64).bin("long", 5..=32u64));
    // The skid buffer never drops a beat: tvalid on the input while the
    // buffer is full implies tready low.
    // With at most three cycles of sink backpressure per beat and a
    // one-entry skid buffer, an offered beat is accepted within 8 cycles.
    let _accepted = assert_implies(
        "input_accepted",
        clk,
        move || s.tvalid.get_u64_lossy() == 1,
        move || s.tvalid.get_u64_lossy() == 1 && s.tready.get_u64_lossy() == 1,
        8,
    );
    let _never = assert_never("tkeep_zero_on_valid", clk, move || {
        mo.tvalid.get_u64_lossy() == 1 && mo.tkeep.unwrap().get_u64_lossy() == 0
    });
    let mut sb = ModelScoreboard::new("packets", |p: &Vec<u8>| Some(p.clone()));
    let mut rng = rivet::rng();
    let packets: Vec<Vec<u8>> = (0..25)
        .map(|_| {
            let n = match rng.gen_range(0..3u8) {
                0 => 1,
                1 => rng.gen_range(2..=4usize),
                _ => rng.gen_range(5..=32usize),
            };
            (0..n).map(|_| rng.gen()).collect()
        })
        .collect();
    for p in &packets {
        len_pt.sample(p.len() as u64);
        sb.drive(p.clone());
    }
    let sender = spawn(async move {
        for p in &packets {
            src.send_packet(p).await;
        }
    });
    let observe = sb.scoreboard();
    for _ in 0..25 {
        observe.observe(sink.recv_packet().await);
    }
    let _ = sender.await;
    sb.finish()?;
    ensure!(cg.percent() == 100.0, "packet length coverage {:.0}%", cg.percent());
    Ok(())
}

#[rivet::test]
async fn apb_regfile(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let mut m = ApbMaster::new(Apb::find(&dut, clk, "apb_")?);
    m.write(0x0, 0x1234_5678).await?;
    m.write(0xc, 0xdead_beef).await?;
    assert_eq!(m.read(0x0).await?, 0x1234_5678);
    assert!(!m.write_strb(0x0, 0xffff_ffff, 0b0001).await);
    assert_eq!(m.read(0x0).await?, 0x1234_56ff);
    assert_eq!(m.read(0xc).await?, 0xdead_beef);
    ensure!(m.write(0x40, 1).await.is_err(), "PSLVERR expected");
    ensure!(m.read(0x44).await.is_err());
    Ok(())
}

#[rivet::test]
async fn wishbone_regfile(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let mut m = WishboneMaster::new(Wishbone::find(&dut, clk, "wb_")?);
    for i in 0..4u64 {
        m.write(i, 0x1000 + i).await?;
    }
    m.write_sel(1, 0xffff_ffff, 0b1000).await?;
    assert_eq!(m.read(1).await?, 0xff00_1001);
    assert_eq!(m.read(3).await?, 0x1003);
    ensure!(m.write(0x40, 1).await.is_err(), "ERR expected");
    Ok(())
}

#[rivet::test]
async fn avalon_regfile(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let mut m = AvalonMaster::new(Avalon::find(&dut, clk, "av_")?);
    for i in 0..4u64 {
        m.write(i * 4, 0x2200 + i).await;
    }
    m.write_be(0x8, 0xffff_ffff, 0b0010).await;
    for i in 0..4u64 {
        // Byte enable 0b0010 replaced bits 15:8 of register 2 with 0xff.
        let expect = if i == 2 { 0xff02 } else { 0x2200 + i };
        assert_eq!(m.read(i * 4).await, expect, "reg {i}");
    }
    Ok(())
}

/// Runs only under the `init5` parameter set: registers reset to 5.
#[rivet::test(param_sets = ["init5"])]
async fn regs_reset_to_param(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let mut m = AxiLiteMaster::new(AxiLite::find(&dut, clk, "s_axil_")?);
    assert_eq!(rivet::test::param_set().as_deref(), Some("init5"));
    assert_eq!(dut.signal("REG_INIT")?.get_u64()?, 5);
    for i in 0..8u64 {
        assert_eq!(m.read(i * 4).await?, 5, "reg {i} after reset");
    }
    Ok(())
}

/// The ALU through the typed bindings: `Cmd` and `Op` come from the
/// design's `typedef`s, so an operand out of range is a compile error and
/// a value outside the enum is a bind-time error.
#[rivet::test]
async fn alu_typed_cmd(dut: Dut) -> rivet::Result<()> {
    let _clock = Clock::start(dut.clk, 10.ns());
    Reset::new(dut.clk, dut.rst_n).active_low().apply().await;
    ensure!(dut.hierarchy().contains("u_axil_slice"), "{}", dut.hierarchy());
    assert_eq!(Cmd::WIDTH, 18);
    let mut sb = ModelScoreboard::new("alu", |c: &Cmd| {
        Some(match c.op {
            Op::OP_NOP => 0u64,
            Op::OP_ADD => c.a + c.b,
            Op::OP_SUB => (c.a.wrapping_sub(c.b)) & 0xffff,
            Op::OP_MUL => c.a * c.b,
        })
    });
    let mut rng = rivet::rng();
    let results = sb.scoreboard();
    for _ in 0..40 {
        let cmd = Cmd { op: *rng.choose(Op::ALL).unwrap(), a: rng.gen_range(0..256u64), b: rng.gen_range(0..256u64) };
        cmd.set_on(&dut.cmd);
        sb.drive(cmd.clone());
        // The ALU registers the result on the edge that samples the command.
        dut.clk.rising_edge().await;
        read_only().await;
        assert_eq!(Cmd::from_signal(&dut.cmd)?, cmd, "packed struct round trip");
        results.observe(dut.alu_out.get_u64()?);
        dut.clk.falling_edge().await;
    }
    sb.finish()?;
    // An out-of-range enum value is reported, not silently mapped.
    ensure!(Op::from_bits(7).is_none());
    assert_eq!(Op::OP_MUL.name(), "OP_MUL");
    Ok(())
}

/// Stability and window checks, plus waveform control around the
/// interesting part.
#[rivet::test(wall_timeout = 60)]
async fn checkers_and_waves(dut: Module) -> rivet::Result<()> {
    let clk = reset(&dut).await?;
    let alu_out = dut.signal("alu_out")?;
    let cmd = dut.signal("cmd")?;
    cmd.set(0);
    clk.rising_edge().await;
    assert_stable(clk, alu_out, 5).await?;
    rivet::waves::start(Some("alu_window"));
    // OP_ADD = 1 in bits [17:16], a = 3, b = 4.
    cmd.set((1u64 << 16) | (3 << 8) | 4);
    rivet::kit::assert_becomes(clk, alu_out, 7, 3).await?;
    rivet::waves::off();
    let dump = rivet::dump_tasks();
    ensure!(dump.contains("clock(bus_top.clk)"), "{dump}");
    Ok(())
}
