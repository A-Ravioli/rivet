//! FIFO testbench using the kit: a valid/ready source driven from a queue,
//! a sink with random backpressure, and a scoreboard.

use rivet::kit::{drive_from_queue, monitor_to_queue, reset, Scoreboard, ValidReadySink, ValidReadySource};
use rivet::prelude::*;

struct Ports {
    clk: Signal,
    rst_n: Signal,
    in_valid: Signal,
    in_ready: Signal,
    in_data: Signal,
    out_valid: Signal,
    out_ready: Signal,
    out_data: Signal,
}

fn ports(dut: &Module) -> rivet::Result<Ports> {
    Ok(Ports {
        clk: dut.signal("clk")?,
        rst_n: dut.signal("rst_n")?,
        in_valid: dut.signal("in_valid")?,
        in_ready: dut.signal("in_ready")?,
        in_data: dut.signal("in_data")?,
        out_valid: dut.signal("out_valid")?,
        out_ready: dut.signal("out_ready")?,
        out_data: dut.signal("out_data")?,
    })
}

/// A small deterministic PRNG so runs are reproducible.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

async fn run_traffic(dut: Module, n: usize, ready_policy: impl FnMut(u64) -> bool + 'static) -> rivet::Result<()> {
    let p = ports(&dut)?;
    let _clock = Clock::start(p.clk, 10.ns());
    p.in_valid.set(0);
    p.out_ready.set(0);
    reset(p.clk, p.rst_n, true, 2).await;

    let scoreboard: Scoreboard<u64> = Scoreboard::new("fifo");
    let source = ValidReadySource::new(p.clk, p.in_valid, p.in_ready, p.in_data);
    let sink = ValidReadySink::with_ready(p.clk, p.out_valid, p.out_ready, p.out_data, ready_policy);

    let to_send: Queue<u64> = Queue::new();
    let received: Queue<LogicVec> = Queue::new();
    let driver = drive_from_queue("source", source, to_send.clone());
    let monitor = monitor_to_queue("sink", sink, received.clone());

    let mut rng = Lcg(0x5eed);
    for _ in 0..n {
        let v = rng.next() & 0xffff;
        scoreboard.expect(v);
        to_send.put(v).await;
    }
    for _ in 0..n {
        let got = with_timeout(received.get(), 10.us()).await?;
        scoreboard.observe(got.to_u64()?);
    }
    driver.cancel();
    monitor.cancel();
    p.clk.rising_edge().await;
    read_only().await;
    ensure!(p.out_valid.get_u64()? == 0, "FIFO should be empty at the end");
    scoreboard.finish()
}

#[rivet::test(timeout = 1.ms())]
async fn streams_with_always_ready(dut: Module) -> rivet::Result<()> {
    run_traffic(dut, 200, |_| true).await
}

#[rivet::test(timeout = 1.ms())]
async fn streams_with_backpressure(dut: Module) -> rivet::Result<()> {
    let mut rng = Lcg(42);
    run_traffic(dut, 200, move |_| !rng.next().is_multiple_of(3)).await
}

#[rivet::test(timeout = 1.ms())]
async fn fills_and_stalls(dut: Module) -> rivet::Result<()> {
    let p = ports(&dut)?;
    let _clock = Clock::start(p.clk, 10.ns());
    p.in_valid.set(0);
    p.out_ready.set(0);
    reset(p.clk, p.rst_n, true, 2).await;
    let mut source = ValidReadySource::new(p.clk, p.in_valid, p.in_ready, p.in_data);
    for i in 0..4u64 {
        with_timeout(source.send_value(i), 100.ns()).await?;
    }
    // Fifth write must stall while nobody reads.
    let r = with_timeout(source.send_value(99u64), 100.ns()).await;
    ensure!(r.is_err(), "write into a full FIFO should stall");
    read_only().await;
    ensure!(p.in_ready.get_u64()? == 0, "in_ready must be low when full");
    ensure!(p.out_valid.get_u64()? == 1);
    ensure!(p.out_data.get_u64()? == 0, "first word out");
    Ok(())
}
