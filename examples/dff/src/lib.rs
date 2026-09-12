//! Rivet port of cocotb's `dff` example, plus a few extra checks.

use rivet::prelude::*;

async fn reset(dut: &Module, clk: Signal) -> rivet::Result<()> {
    let rst_n = dut.signal("rst_n")?;
    rst_n.set(0);
    dut.signal("d")?.set(0);
    clk.rising_edge().await;
    clk.rising_edge().await;
    rst_n.set(1);
    clk.rising_edge().await;
    Ok(())
}

#[rivet::test(timeout = 100.us())]
async fn dff_follows_d(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let d = dut.signal("d")?;
    let q = dut.signal("q")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    let mut prev_q = 0u64;
    let mut last_d = 0u64;
    for i in 0..50u64 {
        let v = (i * 37 + 11) & 0xff;
        clk.rising_edge().await;
        // Values-change phase: the flop has not updated yet.
        assert_eq!(q.get_u64()?, prev_q, "cycle {i}: q before update");
        // Deposit the next value; it is not sampled until the next edge.
        d.set(v);
        read_only().await;
        assert_eq!(q.get_u64()?, last_d, "cycle {i}: q after update");
        prev_q = last_d;
        last_d = v;
    }
    info!("dff_follows_d done at {}ns", rivet::now_in(rivet::Unit::Ns));
    Ok(())
}

#[rivet::test]
async fn counter_counts(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let count = dut.signal("count")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    read_only().await;
    let base = count.get_u64()?;
    for i in 1..=20u64 {
        clk.rising_edge().await;
        read_only().await;
        assert_eq!(count.get_u64()?, base + i);
    }
    Ok(())
}

#[rivet::test]
async fn parameters_and_hierarchy(dut: Module) -> rivet::Result<()> {
    let width = dut.signal("WIDTH")?;
    ensure!(width.is_const(), "WIDTH should be a parameter");
    assert_eq!(width.get_u64()?, 8);
    assert_eq!(dut.signal("q")?.width(), 8);
    assert_eq!(dut.signal("sum")?.width(), 9);
    let names: Vec<String> = dut.children()?.iter().map(|o| o.name()).collect();
    info!("children of {}: {:?}", dut.path(), names);
    for n in ["clk", "rst_n", "d", "q", "count", "sum"] {
        ensure!(names.iter().any(|x| x == n), "missing child {n}");
    }
    Ok(())
}

#[rivet::test]
async fn x_propagates_before_reset(dut: Module) -> rivet::Result<()> {
    let q = dut.signal("q")?;
    let v = q.get();
    info!("q at start = {v}");
    if rivet::runtime::caps().four_state {
        ensure!(!v.is_resolvable(), "q should be X before reset on a 4-state simulator, got {v}");
        ensure!(q.get_u64().is_err(), "X must not convert to an integer");
    }
    Ok(())
}

#[rivet::test(expect_fail)]
async fn deliberate_failure(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    clk.rising_edge().await;
    bail!("this test fails on purpose")
}

#[rivet::test]
async fn concurrent_tasks(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let d = dut.signal("d")?;
    let q = dut.signal("q")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    let q_values: Queue<u64> = Queue::new();
    let qv = q_values.clone();
    let monitor = spawn(async move {
        loop {
            clk.rising_edge().await;
            read_only().await;
            qv.put(q.get_u64().unwrap()).await;
        }
    });
    let mut sent = Vec::new();
    for i in 0..10u64 {
        d.set(i + 100);
        sent.push(i + 100);
        clk.rising_edge().await;
    }
    let mut seen = Vec::new();
    while seen.len() < 10 {
        seen.push(q_values.get().await);
    }
    monitor.cancel();
    assert_eq!(seen, sent);
    Ok(())
}

#[rivet::test(timeout = 50.ns(), expect_fail)]
async fn times_out(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    loop {
        clk.rising_edge().await;
    }
}
