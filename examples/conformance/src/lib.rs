//! Conformance suite: every backend feature on a real simulator, with the
//! expected behaviour written down per simulator capability. Run on every
//! supported simulator; a difference is either a Rivet bug or a documented
//! simulator quirk.

use rivet::prelude::*;
use rivet::runtime;
use rivet::ObjKind;

fn sim_name() -> String {
    runtime::backend(|b| b.name().to_string())
}

fn four_state() -> bool {
    runtime::caps().four_state
}

async fn reset(dut: &Module, clk: Signal) -> rivet::Result<()> {
    let rst_n = dut.signal("rst_n")?;
    rst_n.set(0);
    dut.signal("din")?.set(0);
    dut.signal("wide_in")?.set(0u64);
    dut.signal("sin")?.set(0);
    dut.signal("addr")?.set(0);
    clk.rising_edge().await;
    clk.rising_edge().await;
    rst_n.set(1);
    clk.rising_edge().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// cocotb's inertial-write cases (tests/test_cases/test_inertial_writes)

/// A deposit made before awaiting an edge on the same signal is not yet
/// applied when the edge trigger registers, so the edge fires.
#[rivet::test(timeout = 1.us())]
async fn writes_on_timer_seen_on_edge(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    clk.set(0);
    Timer::new(10.ns()).await;
    clk.set(1);
    with_timeout(clk.rising_edge(), 10.ns()).await?;
    Ok(())
}

/// A deposit is handed to the simulator at the start of the ReadWrite
/// phase. Whether it is readable back in that same phase depends on the
/// simulator (cocotb marks this expect_fail on most); by ReadOnly it is
/// always visible.
#[rivet::test(timeout = 1.us())]
async fn read_back_in_readwrite(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    clk.set(0);
    Timer::new(10.ns()).await;
    assert_eq!(clk.get_u64()?, 0);
    clk.set(1);
    assert_eq!(clk.get_u64()?, 0, "not visible before the simulator evaluates");
    read_write().await;
    let in_rw = clk.get_u64()?;
    info!("{}: deposit visible in the same ReadWrite phase: {}", sim_name(), in_rw == 1);
    read_only().await;
    assert_eq!(clk.get_u64()?, 1, "visible by ReadOnly");
    Ok(())
}

/// A deposit made at an edge is not sampled by that edge's flops.
#[rivet::test(timeout = 1.us())]
async fn writes_dont_update_hdl_this_delta(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let din = dut.signal("din")?;
    let dout = dut.signal("dout_reg")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    din.set(0);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(dout.get_u64()?, 0);
    clk.rising_edge().await;
    din.set(1);
    read_only().await;
    assert_eq!(dout.get_u64()?, 0, "write at the edge is not used by that edge");
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(dout.get_u64()?, 1, "used by the next edge");
    Ok(())
}

/// A write in the ReadWrite phase is applied before ReadOnly, even when a
/// scheduled write already caused this ReadWrite.
#[rivet::test(timeout = 1.us())]
async fn writes_in_read_write(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let din = dut.signal("din")?;
    let _clock = Clock::start(clk, 10.ns());
    din.set(0);
    clk.rising_edge().await;
    assert_eq!(din.get_u64()?, 0);
    din.set(5);
    read_write().await;
    din.set(1);
    read_only().await;
    assert_eq!(din.get_u64()?, 1);
    Ok(())
}

/// Same, when the ReadWrite phase had no pending writes (the simulator may
/// think the evaluation loop is over).
#[rivet::test(timeout = 1.us())]
async fn writes_in_last_read_write(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let din = dut.signal("din")?;
    let _clock = Clock::start(clk, 10.ns());
    din.set(0);
    clk.rising_edge().await;
    read_write().await;
    din.set(1);
    read_only().await;
    assert_eq!(din.get_u64()?, 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Triggers and phases

#[rivet::test(timeout = 1.us())]
async fn next_time_step_and_timer_precision(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    clk.rising_edge().await;
    let t = rivet::now();
    next_time_step().await;
    assert_eq!(rivet::now() - t, 5_000, "next time step is the falling edge, 5ns = 5000ps");
    Timer::new(2.5.ns()).await;
    assert_eq!(rivet::now() - t, 7_500);
    Timer::steps(1).await;
    assert_eq!(rivet::now() - t, 7_501);
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn combinational_visible_after_read_write(dut: Module) -> rivet::Result<()> {
    let din = dut.signal("din")?;
    let comb = dut.signal("dout_comb")?;
    Timer::new(1.ns()).await;
    din.set(5);
    read_write().await;
    read_write().await; // the deposit applied at the first ReadWrite propagates in a new cycle
    assert_eq!(comb.get_u64()?, 6);
    Timer::new(1.ns()).await;
    din.set(9);
    read_only().await;
    assert_eq!(comb.get_u64()?, 10);
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn falling_edge_and_value_change(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let din = dut.signal("din")?;
    let _clock = Clock::start(clk, 10.ns());
    clk.rising_edge().await;
    let t = rivet::now();
    clk.falling_edge().await;
    assert_eq!(rivet::now(), t + 5_000);
    let watcher = spawn(async move {
        din.value_change().await;
        rivet::now()
    });
    Timer::new(3.ns()).await;
    din.set(0x33);
    let changed_at = watcher.await?;
    assert!(changed_at >= t + 5_000 + 3_000, "value change seen after the deposit landed: {changed_at}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Values

#[rivet::test(stage = -1)]
async fn x_before_reset(dut: Module) -> rivet::Result<()> {
    let v = dut.signal("dout_reg")?.get();
    let w = dut.signal("wide_out")?.get();
    if four_state() {
        ensure!(!v.is_resolvable() && !w.is_resolvable(), "{v} {w}");
        ensure!(v.has_x() && !v.has_z());
    } else {
        info!("{} is two-state: dout_reg = {v}", sim_name());
    }
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn wide_vectors(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let win = dut.signal("wide_in")?;
    let wout = dut.signal("wide_out")?;
    assert_eq!(win.width(), 128);
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    let pattern = LogicVec::parse("128'hDEADBEEF_CAFEBABE_0123456789ABCDEF_FF").unwrap();
    let pattern = pattern.into_logic_vec(128);
    win.set(&pattern);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(wout.get(), pattern);
    assert_eq!(wout.get().slice(31, 0).to_u64()?, 0xABCD_EFFF);
    // u128 round trip (leave ReadOnly before writing).
    clk.rising_edge().await;
    win.set(u128::MAX - 1);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(wout.get().to_u128()?, u128::MAX - 1);
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn signed_values(dut: Module) -> rivet::Result<()> {
    let sin = dut.signal("sin")?;
    let sneg = dut.signal("sneg")?;
    Timer::new(1.ns()).await;
    sin.set(-5i16);
    read_only().await;
    assert_eq!(sin.get_i64()?, -5);
    assert_eq!(sin.get_u64()?, 0xfffb);
    assert_eq!(sneg.get_i64()?, 5);
    Timer::new(1.ns()).await;
    sin.set(0x7fffu16);
    read_only().await;
    assert_eq!(sneg.get_i64()?, -0x7fff);
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn packed_struct_as_vector(dut: Module) -> rivet::Result<()> {
    let din = dut.signal("din")?;
    let pr = dut.signal("pr")?;
    assert_eq!(pr.width(), 8, "packed struct is an 8-bit vector");
    Timer::new(1.ns()).await;
    din.set(0xa5);
    read_only().await;
    assert_eq!(pr.get_u64()?, 0xa5);
    match pr.member("hi") {
        Ok(hi) => {
            assert_eq!(hi.get_u64()?, 0xa);
            info!("{}: packed struct members are addressable", sim_name());
        }
        Err(e) => info!("{}: packed struct members not addressable ({e})", sim_name()),
    }
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn string_integer_real(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    match dut.signal("s") {
        Ok(s) => {
            assert_eq!(s.kind(), ObjKind::String);
            assert_eq!(s.get_string(), "init");
            s.set_string("changed");
            read_only().await;
            assert_eq!(s.get_string(), "changed");
            clk.rising_edge().await;
        }
        // Icarus does not expose `string` variables through VPI at all.
        Err(e) => info!("{}: string variable not visible ({e})", sim_name()),
    }

    let cnt = dut.signal("cnt")?;
    let acc = dut.signal("acc")?;
    assert_eq!(acc.kind(), ObjKind::Real);
    clk.rising_edge().await;
    read_only().await;
    let c0 = cnt.get_i64()?;
    let a0 = acc.get_real();
    for _ in 0..4 {
        clk.rising_edge().await;
    }
    read_only().await;
    assert_eq!(cnt.get_i64()?, c0 + 4);
    assert!((acc.get_real() - (a0 + 1.0)).abs() < 1e-9);
    clk.falling_edge().await;
    acc.set_real(-3.75);
    read_only().await;
    assert!((acc.get_real() + 3.75).abs() < 1e-9);
    clk.falling_edge().await;
    cnt.set_int(-100);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(cnt.get_i64()?, -99);
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn memory_array(dut: Module) -> rivet::Result<()> {
    let mem = dut.signal("mem")?;
    let addr = dut.signal("addr")?;
    let out = dut.signal("mem_out")?;
    assert_eq!(mem.kind(), ObjKind::Array);
    assert_eq!(mem.width(), 4);
    for i in 0..4 {
        mem.index(i)?.set(0xf0 + i as u64);
    }
    ensure!(mem.index(4).is_err(), "out of range index");
    for i in 0..4 {
        Timer::new(1.ns()).await;
        addr.set(i as u64);
        read_only().await;
        assert_eq!(out.get_u64()?, 0xf0 + i as u64, "mem[{i}]");
        assert_eq!(mem.index(i)?.get_u64()?, 0xf0 + i as u64);
    }
    Ok(())
}

#[rivet::test]
async fn parameters(dut: Module) -> rivet::Result<()> {
    let w = dut.signal("W")?;
    assert!(w.is_const());
    assert_eq!(w.get_u64()?, 8);
    let rp = dut.signal("RP")?;
    assert_eq!(rp.kind(), ObjKind::Real, "{}: real parameter kind", sim_name());
    assert!((rp.get_real() - 1.5).abs() < 1e-12);
    match dut.signal("SP") {
        Ok(sp) => {
            info!("{}: string parameter kind {:?} value {:?}", sim_name(), sp.kind(), sp.read_raw());
        }
        Err(e) => info!("{}: string parameter not visible ({e})", sim_name()),
    }
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn generate_instances(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let din = dut.signal("din")?;
    let _clock = Clock::start(clk, 10.ns());
    din.set(0x10);
    clk.rising_edge().await;
    clk.rising_edge().await;
    read_only().await;
    for i in 0..2 {
        let r = dut.path_signal(&format!("blk[{i}].r"))?;
        assert_eq!(r.get_u64()?, 0x10 ^ i as u64);
    }
    let blk = dut.module("blk")?;
    assert!(blk.index(2).is_err());
    Ok(())
}

#[rivet::test(timeout = 1.us())]
async fn force_and_release(dut: Module) -> rivet::Result<()> {
    if !runtime::caps().supports_force {
        info!("{}: force/release not supported; skipping", sim_name());
        return Ok(());
    }
    let din = dut.signal("din")?;
    let comb = dut.signal("dout_comb")?;
    Timer::new(1.ns()).await;
    din.set(1);
    read_only().await;
    assert_eq!(comb.get_u64()?, 2);
    Timer::new(1.ns()).await;
    comb.force(0x55);
    din.set(7);
    read_only().await;
    assert_eq!(comb.get_u64()?, 0x55, "forced value holds against the driver");
    Timer::new(1.ns()).await;
    comb.release();
    din.set(3);
    read_only().await;
    assert_eq!(comb.get_u64()?, 4, "driver takes over after release");
    Ok(())
}

// ---------------------------------------------------------------------------
// Hierarchy, argv, tasks

#[rivet::test]
async fn discovery(dut: Module) -> rivet::Result<()> {
    let names: Vec<String> = dut.children()?.iter().map(|o| o.name()).collect();
    for n in ["clk", "din", "dout_reg", "wide_in", "sin", "mem", "cnt", "acc", "pr", "W"] {
        ensure!(names.iter().any(|x| x == n), "{}: missing {n} in {names:?}", sim_name());
    }
    ensure!(dut.signal("no_such").is_err());
    ensure!(dut.path_signal("blk[0].nope").is_err());
    assert_eq!(dut.name(), "conformance");
    assert_eq!(dut.signal("clk")?.path(), "conformance.clk");
    info!("{}: string variable visible: {}", sim_name(), names.iter().any(|x| x == "s"));
    Ok(())
}

#[rivet::test]
async fn plusargs_reach_the_harness(_dut: Module) -> rivet::Result<()> {
    let argv = runtime::backend(|b| b.argv());
    info!("{}: argv = {argv:?}", sim_name());
    ensure!(argv.iter().any(|a| a == "+conf=1"), "run_args from rivet.toml should be visible");
    Ok(())
}

#[rivet::test(timeout = 5.us())]
async fn cancelled_tasks_leave_no_callbacks_behind(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let din = dut.signal("din")?;
    let _clock = Clock::start(clk, 10.ns());
    yield_now().await; // let the clock task arm its first timer
    let timers_before = runtime::debug_pending_timers(); // the clock's own timer
    let mut hs = Vec::new();
    for i in 0..50u64 {
        hs.push(spawn(async move {
            loop {
                if i % 2 == 0 {
                    clk.rising_edge().await;
                } else {
                    Timer::new((i + 1).ns()).await;
                }
                din.set(i);
            }
        }));
    }
    clk.rising_edge().await;
    clk.rising_edge().await;
    for h in &hs {
        h.cancel();
    }
    assert_eq!(runtime::debug_pending_timers(), timers_before, "cancelled tasks' timers removed");
    assert_eq!(runtime::debug_edge_waiters(clk.handle()), 0);
    for _ in 0..20 {
        clk.rising_edge().await;
    }
    Ok(())
}

/// With `+finish_early` the HDL calls `$finish` at 55ns; the harness must
/// report the running test as failed rather than hang or crash.
#[rivet::test(stage = 9)]
async fn hdl_finish_is_reported(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    Timer::new(100.ns()).await;
    Ok(())
}
