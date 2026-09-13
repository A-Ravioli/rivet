//! VHDL flip-flop on GHDL (through GHDL's VPI).

use rivet::prelude::*;

#[rivet::test(timeout = 10.us())]
async fn dff_follows_d(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let rst_n = dut.signal("rst_n")?;
    let d = dut.signal("d")?;
    let q = dut.signal("q")?;
    let _clock = Clock::start(clk, 10.ns());
    rst_n.set(0);
    d.set(0);
    clk.rising_edge().await;
    clk.rising_edge().await;
    rst_n.set(1);
    clk.rising_edge().await;
    let mut last_d = 0u64;
    for i in 0..20u64 {
        let v = (i * 29 + 3) & 0xff;
        clk.rising_edge().await;
        d.set(v);
        read_only().await;
        assert_eq!(q.get_u64()?, last_d, "cycle {i}");
        last_d = v;
    }
    Ok(())
}

#[rivet::test]
async fn counter_and_types(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let count = dut.signal("count")?;
    let _clock = Clock::start(clk, 10.ns());
    dut.signal("rst_n")?.set(0);
    clk.rising_edge().await;
    dut.signal("rst_n")?.set(1);
    clk.rising_edge().await;
    read_only().await;
    let base = count.get_u64()?;
    for i in 1..=5u64 {
        clk.rising_edge().await;
        read_only().await;
        assert_eq!(count.get_u64()?, base + i);
    }
    info!("count width {}, generic WIDTH = {:?}", count.width(), dut.signal("WIDTH").map(|s| s.get_u64()));
    let names: Vec<String> = dut.children()?.iter().map(|o| format!("{}:{:?}", o.name(), o.kind())).collect();
    info!("children: {names:?}");
    Ok(())
}
