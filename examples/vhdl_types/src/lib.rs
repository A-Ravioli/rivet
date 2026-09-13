//! VHDL types through VHPI (NVC) and VPI (GHDL): an enumeration, a record
//! port, a generic, a for-generate region, booleans and integers.

use rivet::prelude::*;

/// GHDL's VPI does not expose VHDL record members, so tests that need them
/// skip themselves there rather than failing.
fn need_records(dut: &Module) -> rivet::Result<Module> {
    dut.module("cmd").map_err(|_| rivet::skip("this simulator does not expose VHDL record members"))
}

async fn reset(dut: &Module, clk: Signal) -> rivet::Result<()> {
    dut.signal("rst_n")?.set(0);
    // Deposits persist between tests, so every test starts from known
    // inputs rather than whatever the last one left behind.
    if let Ok(cmd) = dut.module("cmd") {
        cmd.signal("valid")?.set(0);
        cmd.signal("op")?.set(0u64);
        cmd.signal("data")?.set(0);
    }
    dut.signal("b")?.set(0);
    clk.rising_edge().await;
    clk.rising_edge().await;
    dut.signal("rst_n")?.set(1);
    clk.rising_edge().await;
    Ok(())
}

#[rivet::test(timeout = 100.us())]
async fn clock_and_reset_drive_internal_signals(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    // `cycles` is a VHDL integer driven by the design.
    let cycles = dut.signal("cycles")?;
    // Sample in the same phase at both ends: an edge returns before the
    // design's process has run, so a values-change read is one behind.
    read_only().await;
    let before = cycles.get_u64()?;
    next_time_step().await;
    for _ in 0..5 {
        clk.rising_edge().await;
    }
    read_only().await;
    let after = cycles.get_u64()?;
    assert_eq!(after - before, 5, "the design counted the clocks we drove");
    // `armed` is a VHDL boolean: one bit, FALSE then TRUE.
    assert_eq!(dut.signal("armed")?.get_u64()?, 1, "armed after reset");
    Ok(())
}

/// Enumeration literals: a VHDL tool reports them, so a test can say what
/// a value means rather than which position it holds.
#[rivet::test(timeout = 100.us())]
async fn enum_literals_have_names(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    let cmd = need_records(&dut)?;
    let op = cmd.signal("op")?;
    let Some(lits) = op.enum_literals() else {
        return Err(rivet::skip("this simulator does not report enumeration literals"));
    };
    assert_eq!(lits, ["OP_NOP", "OP_ADD", "OP_SUB", "OP_XOR"], "op_t literals in order");
    op.set(2u64);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(op.enum_name().as_deref(), Some("OP_SUB"));
    // The design's own signal follows it a cycle later.
    let last = dut.module("dut")?.signal("last_op")?;
    assert_eq!(last.enum_name().as_deref(), Some("OP_SUB"), "the ALU registered the operation");
    Ok(())
}

#[rivet::test(timeout = 100.us())]
async fn record_members_are_addressable(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;

    // A record port: members are children of the record object.
    let cmd = need_records(&dut)?;
    let op = cmd.signal("op")?;
    let valid = cmd.signal("valid")?;
    let data = cmd.signal("data")?;
    assert_eq!(data.width(), 8, "record member width");

    // OP_ADD is literal 1 of op_t.
    op.set(1u64);
    valid.set(1);
    data.set(0x5a);
    dut.signal("b")?.set(7);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(dut.signal("result")?.get_u64()?, 7, "add put 7 in the accumulator");
    assert_eq!(dut.signal("done")?.get_u64()?, 1, "done follows cmd.valid");
    next_time_step().await;

    // Subtract it again.
    op.set(2u64);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(dut.signal("result")?.get_u64()?, 0, "sub returned to zero");
    Ok(())
}

/// Structure only: works on every VHDL simulator.
#[rivet::test(timeout = 100.us())]
async fn generate_region_elements_exist(dut: Module) -> rivet::Result<()> {
    // The DUT is an instance inside the testbench; the generate block is a
    // region inside it, with one element per tap. Neither simulator exposes
    // the generate label itself as an object, so it is a pseudo-region.
    let alu = dut.module("dut")?;
    let taps = alu.module("tapgen")?;
    for i in 0..4 {
        let tap = taps.index(i)?.as_module()?.signal("tap")?;
        assert_eq!(tap.width(), 1, "tapgen[{i}].tap");
    }
    // The generic reached the instance: TAPS = 4, so there is no fifth.
    assert!(taps.index(4).is_err(), "only four taps were generated");
    Ok(())
}

#[rivet::test(timeout = 100.us())]
async fn generate_region_follows_the_accumulator(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;

    let alu = dut.module("dut")?;
    let taps = alu.module("tapgen")?;
    let tap0 = taps.index(0)?.as_module()?.signal("tap")?;

    // Driving the accumulator needs the record port, which GHDL's VPI does
    // not expose.
    let cmd = need_records(&dut)?;
    cmd.signal("op")?.set(1u64);
    cmd.signal("valid")?.set(1);
    dut.signal("b")?.set(1);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(tap0.get_u64()?, 1, "tap 0 follows accumulator bit 0");
    Ok(())
}

/// A discovery aid: dump what the simulator exposes under the DUT. Enabled
/// with RIVET_LIST_CHILDREN=1 so it does not run in a normal regression.
#[rivet::test]
async fn list_children(dut: Module) -> rivet::Result<()> {
    if std::env::var("RIVET_LIST_CHILDREN").is_err() {
        return Err(rivet::skip("set RIVET_LIST_CHILDREN=1 to dump the hierarchy"));
    }
    for c in dut.children()? {
        info!("tb_top child {} ({:?})", c.name(), c.kind());
    }
    let alu = dut.module("dut")?;
    for c in alu.children()? {
        info!("dut child {} ({:?})", c.name(), c.kind());
    }
    Ok(())
}
