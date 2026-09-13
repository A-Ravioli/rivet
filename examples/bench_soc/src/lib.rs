//! Harness overhead on a real design: PicoRV32 running a two-instruction
//! loop out of a memory. The interesting number is not microseconds per
//! cycle on an empty design, it is what fraction of a real simulation the
//! harness costs.
//!
//! `RIVET_BENCH_N` sets the cycle count (default 100000).

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

async fn release_reset(dut: &Module, clk: Signal) -> rivet::Result<()> {
    let resetn = dut.signal("resetn")?;
    resetn.set(0);
    for _ in 0..4 {
        clk.rising_edge().await;
    }
    resetn.set(1);
    clk.rising_edge().await;
    Ok(())
}

/// The harness drives the clock and does nothing else: the design's own
/// evaluation dominates.
#[rivet::test(wall_timeout = 600.0)]
async fn cpu_clock_only(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    release_reset(&dut, clk).await?;
    let cycles = n();
    let t = Instant::now();
    for _ in 0..cycles {
        clk.rising_edge().await;
    }
    report("soc_clock_only", cycles, t);
    Ok(())
}

/// A realistic testbench: a task watching the core's memory interface every
/// cycle and counting instruction fetches, the way a monitor would.
#[rivet::test(wall_timeout = 600.0)]
async fn cpu_with_bus_monitor(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    release_reset(&dut, clk).await?;

    let valid = dut.signal("mem_valid")?;
    let ready = dut.signal("mem_ready")?;
    let instr = dut.signal("mem_instr")?;
    let addr = dut.signal("mem_addr")?;
    let cycles = n();
    let t = Instant::now();
    let mut fetches = 0u64;
    let mut last_addr = 0u64;
    for _ in 0..cycles {
        clk.rising_edge().await;
        read_only().await;
        if valid.get_u64_lossy() == 1 && ready.get_u64_lossy() == 1 {
            last_addr = addr.get_u64_lossy();
            if instr.get_u64_lossy() == 1 {
                fetches += 1;
            }
        }
        next_time_step().await;
    }
    report("soc_with_monitor", cycles, t);
    info!("BENCH soc_with_monitor saw {fetches} instruction fetches, last address {last_addr:#x}");
    // The loop is two instructions, so a run of any length fetches plenty.
    assert!(fetches > cycles / 20, "the core kept fetching: {fetches} in {cycles} cycles");
    Ok(())
}
