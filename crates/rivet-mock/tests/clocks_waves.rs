//! Clock phase and jitter, the reset builder, and waveform control.

use rivet_core::backend::WaveCmd;
use rivet_core::test::{boxed, TestDesc};
use rivet_core::triggers::{read_only, Timer};
use rivet_core::{inventory, random, spawn, waves, Clock, LogicVec, Module, TimeExt};
use rivet_kit::Reset;
use rivet_mock::{run_test, wave_log, Design};
use std::cell::RefCell;
use std::rc::Rc;

fn bare() -> Design {
    let mut d = Design::new("top").precision(-9);
    for n in ["clk", "clk2"] {
        let h = d.logic(n, 1);
        d.init(h, LogicVec::from_u64(1, 0));
    }
    d
}

#[test]
fn clock_phase_offsets_the_first_edge() {
    run_test(bare(), |dut| async move {
        let clk = dut.signal("clk")?;
        let clk2 = dut.signal("clk2")?;
        let _a = Clock::start(clk, 10.ns());
        let _b = Clock::builder(clk2, 10.ns()).phase(3.ns()).start();
        clk.rising_edge().await;
        assert_eq!(rivet_core::now(), 0);
        clk2.rising_edge().await;
        assert_eq!(rivet_core::now(), 3);
        clk2.rising_edge().await;
        assert_eq!(rivet_core::now(), 13);
        Ok(())
    })
    .unwrap();
}

fn jittered_edges() -> Vec<u64> {
    let out = Rc::new(RefCell::new(Vec::new()));
    let out2 = out.clone();
    run_test(bare(), move |dut| async move {
        let clk = dut.signal("clk")?;
        let _c = Clock::builder(clk, 10.ns()).jitter(2.ns()).start();
        for _ in 0..200 {
            clk.rising_edge().await;
            out2.borrow_mut().push(rivet_core::now());
        }
        Ok(())
    })
    .unwrap();
    let v = out.borrow().clone();
    v
}

#[test]
fn clock_jitter_is_bounded_reproducible_and_period_preserving() {
    random::set_base_seed(5);
    let a = jittered_edges();
    random::set_base_seed(5);
    let b = jittered_edges();
    assert_eq!(a, b, "same seed, same edges");
    random::set_base_seed(6);
    let c = jittered_edges();
    assert_ne!(a, c, "different seed, different edges");
    let mut moved = 0;
    for (i, w) in a.windows(2).enumerate() {
        let period = w[1] - w[0];
        // Each edge moves by at most 2ns, so a period is within 10 +- 4.
        assert!((6..=14).contains(&period), "period {period} at edge {i}");
        if period != 10 {
            moved += 1;
        }
    }
    assert!(moved > 100, "jitter actually moved edges ({moved} of 199 periods)");
    // No drift: edge k is within one jitter of k * period.
    for (k, t) in a.iter().enumerate() {
        let nominal = 10 * k as i64;
        assert!((*t as i64 - nominal).abs() <= 2, "edge {k} at {t} drifted from {nominal}");
    }
}

fn resettable() -> Design {
    let mut d = Design::new("top").precision(-9);
    let clk = d.logic("clk", 1);
    let rst_n = d.logic("rst_n", 1);
    let arst = d.logic("arst", 1);
    let count = d.logic("count", 8);
    let acount = d.logic("acount", 8);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.init(rst_n, LogicVec::from_u64(1, 1));
    d.init(arst, LogicVec::from_u64(1, 0));
    d.init(count, LogicVec::from_u64(8, 0x55));
    d.init(acount, LogicVec::from_u64(8, 0x55));
    d.clock(clk, 5, 0);
    // Synchronous active-low reset counter.
    d.process(&[clk], move |s| {
        if s.rose(clk) {
            if s.get_u64(rst_n) == 0 {
                s.nba(count, LogicVec::from_u64(8, 0));
            } else {
                s.nba(count, LogicVec::from_u64(8, (s.get_u64(count) + 1) & 0xff));
            }
        }
    });
    // Asynchronous active-high reset counter.
    d.process(&[clk, arst], move |s| {
        if s.get_u64(arst) == 1 {
            s.nba(acount, LogicVec::from_u64(8, 0));
        } else if s.rose(clk) {
            s.nba(acount, LogicVec::from_u64(8, (s.get_u64(acount) + 1) & 0xff));
        }
    });
    d
}

#[test]
fn reset_builder_sync_and_async() {
    run_test(resettable(), |dut| async move {
        let clk = dut.signal("clk")?;
        let rst_n = dut.signal("rst_n")?;
        let arst = dut.signal("arst")?;
        let count = dut.signal("count")?;
        let acount = dut.signal("acount")?;
        let t0 = rivet_core::now();
        Reset::new(clk, rst_n).active_low().cycles(3).settle(1).apply().await;
        // 3 asserted edges + 1 settling edge = 4 rising edges = 40ns.
        assert_eq!(rivet_core::now() - t0, 30, "returned at the fourth rising edge");
        read_only().await;
        assert_eq!(count.get_u64()?, 1, "reset for three edges, counted once after release");
        assert_eq!(rst_n.get_u64()?, 1);

        Timer::new(2.ns()).await;
        let t1 = rivet_core::now();
        Reset::new(clk, arst).asynchronous().cycles(2).settle(2).apply().await;
        // Released on the falling edge after the second asserted edge.
        assert!(rivet_core::now() > t1);
        read_only().await;
        assert_eq!(arst.get_u64()?, 0);
        assert_eq!(acount.get_u64()?, 2, "two settling edges counted after an async release");
        // The legacy helper still works (leave the ReadOnly phase first).
        clk.falling_edge().await;
        rivet_kit::reset(clk, rst_n, true, 2).await;
        read_only().await;
        assert_eq!(count.get_u64()?, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn wave_commands_reach_the_backend() {
    run_test(bare(), |_dut| async move {
        assert!(waves::start(Some("case_a")));
        Timer::steps(1).await;
        assert!(waves::off());
        assert!(waves::on());
        let log = wave_log();
        assert_eq!(log, vec![WaveCmd::File("case_a".into()), WaveCmd::On, WaveCmd::Off, WaveCmd::On]);
        Ok(())
    })
    .unwrap();
}

inventory::submit! {
    TestDesc { name: "wavy", module: "wv", run: |_dut: Module| boxed(async {
        let _t = spawn(async { Timer::steps(2).await; });
        Timer::steps(3).await;
        Ok(())
    }), timeout: || None, skip: false, expect_fail: false, stage: 0, wall_timeout: None, param_sets: &[] }
}

#[test]
fn per_test_wave_files() {
    std::env::set_var("RIVET_WAVES", "per-test");
    let _ = rivet_mock::run_regression(bare(), rivet_core::test::all_tests(), Some("wv::wavy"));
    std::env::remove_var("RIVET_WAVES");
    assert_eq!(wave_log(), vec![WaveCmd::File("wv__wavy".into()), WaveCmd::On, WaveCmd::Off]);
}
