//! Harness overhead benchmarks. Compare with `cocotb/test_bench.py`.
//! Cycle count from `RIVET_BENCH_N` (default 100000).

use rivet::prelude::*;
use std::time::Instant;

fn n() -> u64 {
    std::env::var("RIVET_BENCH_N").ok().and_then(|s| s.parse().ok()).unwrap_or(100_000)
}

fn report(name: &str, cycles: u64, t: Instant) {
    let secs = t.elapsed().as_secs_f64();
    info!(
        "BENCH {name}: {cycles} cycles in {secs:.3}s = {:.0} cycles/s, {:.2} us/cycle",
        cycles as f64 / secs,
        secs * 1e6 / cycles as f64
    );
}

/// One task awaiting every rising edge of a harness-driven clock.
#[rivet::test]
async fn edge_roundtrip(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    let n = n();
    let t = Instant::now();
    for _ in 0..n {
        clk.rising_edge().await;
    }
    report("edge_roundtrip", n, t);
    Ok(())
}

/// Per cycle: write a 32-bit input, read a 32-bit and a 512-bit output.
#[rivet::test]
async fn value_traffic(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let din = dut.signal("din")?;
    let dout = dut.signal("dout")?;
    let wide = dut.signal("wide")?;
    let _clock = Clock::start(clk, 10.ns());
    let n = n();
    let mut wide_buf = LogicVec::zeros(512);
    let mut acc = 0u64;
    let t = Instant::now();
    for i in 0..n {
        clk.rising_edge().await;
        din.set(i as u32);
        acc = acc.wrapping_add(dout.get_u64_lossy());
        wide.read_into(&mut wide_buf);
        acc = acc.wrapping_add(wide_buf.aval()[0] as u64);
    }
    report("value_traffic", n, t);
    ensure!(acc != 0, "sanity");
    Ok(())
}

/// Same as value_traffic but sampling after ReadOnly, the common monitor
/// pattern: two synchronisations per cycle.
#[rivet::test]
async fn edge_then_readonly(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let din = dut.signal("din")?;
    let dout = dut.signal("dout")?;
    let _clock = Clock::start(clk, 10.ns());
    let n = n();
    let t = Instant::now();
    let mut last = 0u64;
    for i in 0..n {
        clk.rising_edge().await;
        din.set(i as u32);
        read_only().await;
        let v = dout.get_u64_lossy();
        if i > 1 {
            ensure!(v == last + 1, "dout {v} != {} at cycle {i}", last + 1);
        }
        last = v;
    }
    report("edge_then_readonly", n, t);
    Ok(())
}

/// 100 monitor tasks each awaiting every clock edge.
#[rivet::test]
async fn many_tasks(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    let n = n() / 10;
    let t = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..100 {
        handles.push(spawn(async move {
            for _ in 0..n {
                clk.rising_edge().await;
            }
        }));
    }
    for h in handles {
        h.await?;
    }
    report("many_tasks(100 tasks)", n, t);
    Ok(())
}

/// Timer path only: no clock, no value-change callbacks.
#[rivet::test]
async fn timer_only(_dut: Module) -> rivet::Result<()> {
    let n = n();
    let t = Instant::now();
    for _ in 0..n {
        Timer::new(10.ns()).await;
    }
    report("timer_only", n, t);
    Ok(())
}

/// Clock running, nobody awaiting edges: measures the clock task itself
/// (two timers and two deposits per period).
#[rivet::test]
async fn clock_only(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    let n = n();
    let t = Instant::now();
    Timer::new((n * 10).ns()).await;
    report("clock_only", n, t);
    Ok(())
}

/// Clock with immediate writes instead of deposits: no ReadWrite flush.
#[rivet::test]
async fn edge_roundtrip_immediate_clock(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::builder(clk, 10.ns()).immediate().start();
    let n = n();
    let t = Instant::now();
    for _ in 0..n {
        clk.rising_edge().await;
    }
    report("edge_roundtrip_immediate_clock", n, t);
    Ok(())
}
