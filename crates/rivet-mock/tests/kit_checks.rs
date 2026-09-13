//! Checkers, the model scoreboard, X detection and golden traces.

use rivet_core::test::{boxed, Outcome, TestDesc};
use rivet_core::triggers::{read_only, Timer};
use rivet_core::{inventory, Logic, LogicVec, Module, TimeExt};
use rivet_kit::check::{
    assert_always, assert_becomes, assert_implies, assert_never, assert_no_x, assert_stable, assert_within, find_x,
};
use rivet_kit::{Model, ModelScoreboard, Trace};
use rivet_mock::{run_test, Design};

/// A counter with an `en` input and a `req`/`ack` pair where `ack` follows
/// `req` two cycles later; `xsig` starts X and is cleared by `fix`.
fn design() -> Design {
    let mut d = Design::new("top").precision(-9);
    let clk = d.logic("clk", 1);
    let en = d.logic("en", 1);
    let count = d.logic("count", 8);
    let req = d.logic("req", 1);
    let d1 = d.logic("d1", 1);
    let ack = d.logic("ack", 1);
    let xsig = d.logic("xsig", 4);
    let fix = d.logic("fix", 1);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.init(en, LogicVec::from_u64(1, 0));
    d.init(count, LogicVec::from_u64(8, 0));
    d.init(req, LogicVec::from_u64(1, 0));
    d.init(d1, LogicVec::from_u64(1, 0));
    d.init(ack, LogicVec::from_u64(1, 0));
    d.init(xsig, LogicVec::xs(4));
    d.init(fix, LogicVec::from_u64(1, 0));
    d.clock(clk, 5, 0);
    d.process(&[clk], move |s| {
        if s.rose(clk) {
            if s.get_u64(en) == 1 {
                s.nba(count, LogicVec::from_u64(8, (s.get_u64(count) + 1) & 0xff));
            }
            s.nba(d1, s.get(req));
            s.nba(ack, s.get(d1));
            if s.get_u64(fix) == 1 {
                s.nba(xsig, LogicVec::from_u64(4, 0xa));
            }
        }
    });
    d
}

#[test]
fn immediate_checkers() {
    run_test(design(), |dut| async move {
        let clk = dut.signal("clk")?;
        let en = dut.signal("en")?;
        let count = dut.signal("count")?;
        clk.rising_edge().await;
        assert_stable(clk, count, 5).await?;
        en.set(1);
        clk.rising_edge().await;
        let e = assert_stable(clk, count, 3).await.unwrap_err().to_string();
        assert!(e.contains("top.count changed from"), "{e}");
        assert_becomes(clk, count, 6, 10).await?;
        let e = assert_becomes(clk, count, 3, 2).await.unwrap_err().to_string();
        assert!(e.contains("did not become 0x3 within 2 cycles"), "{e}");
        assert_within(clk, 20, || count.get_u64_lossy() >= 12).await?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn background_checkers_pass_when_the_design_behaves() {
    run_test(design(), |dut| async move {
        let clk = dut.signal("clk")?;
        let req = dut.signal("req")?;
        let ack = dut.signal("ack")?;
        let count = dut.signal("count")?;
        let en = dut.signal("en")?;
        let _never = assert_never("count_overflow", clk, move || count.get_u64_lossy() > 200);
        let _always = assert_always("count_in_range", clk, move || count.get_u64_lossy() <= 255);
        let _imp =
            assert_implies("req_ack", clk, move || req.get_u64_lossy() == 1, move || ack.get_u64_lossy() == 1, 2);
        en.set(1);
        for _ in 0..5 {
            clk.rising_edge().await;
            req.set(1);
            clk.rising_edge().await;
            req.set(0);
            clk.rising_edge().await;
            clk.rising_edge().await;
        }
        Timer::new(100.ns()).await;
        Ok(())
    })
    .unwrap();
}

inventory::submit! {
    TestDesc { name: "never_fires", module: "chk", run: |dut: Module| boxed(async move {
        let clk = dut.signal("clk")?;
        let count = dut.signal("count")?;
        dut.signal("en")?.set(1);
        let _h = assert_never("count_above_3", clk, move || count.get_u64_lossy() > 3);
        Timer::new(200.ns()).await;
        Ok(())
    }), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "implies_fires", module: "chk", run: |dut: Module| boxed(async move {
        let clk = dut.signal("clk")?;
        let req = dut.signal("req")?;
        let ack = dut.signal("ack")?;
        // ack follows req after two cycles, so a one-cycle window fails.
        let _h = assert_implies("too_tight", clk, move || req.get_u64_lossy() == 1, move || ack.get_u64_lossy() == 1, 1);
        clk.rising_edge().await;
        req.set(1);
        clk.rising_edge().await;
        req.set(0);
        Timer::new(100.ns()).await;
        Ok(())
    }), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "x_detected", module: "chk", run: |dut: Module| boxed(async move {
        let clk = dut.signal("clk")?;
        let xsig = dut.signal("xsig")?;
        let _h = assert_no_x("outputs", clk, vec![dut.signal("count")?, xsig]);
        Timer::new(100.ns()).await;
        Ok(())
    // Runs first: `x_cleared` fixes xsig for the rest of the simulation.
    }), stage: -1, ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "x_cleared", module: "chk", run: |dut: Module| boxed(async move {
        let clk = dut.signal("clk")?;
        let xsig = dut.signal("xsig")?;
        let found = find_x(clk, &[xsig], 2).await;
        assert!(matches!(found, Some((s, _, _)) if s == xsig));
        dut.signal("fix")?.set(1);
        clk.rising_edge().await;
        clk.rising_edge().await;
        let _h = assert_no_x("outputs", clk, vec![dut.signal("count")?, xsig]);
        Timer::new(100.ns()).await;
        assert!(find_x(clk, &[xsig], 3).await.is_none());
        Ok(())
    }), ..TestDesc::DEFAULT }
}

#[test]
fn background_checkers_fail_the_test() {
    let results = rivet_mock::run_regression(design(), rivet_core::test::all_tests(), Some("chk::"));
    let outcome = |n: &str| results.iter().find(|r| r.name == n).unwrap().outcome.clone();
    match outcome("never_fires") {
        Outcome::Failed(m) => assert!(m.contains("assert_never \"count_above_3\" violated at"), "{m}"),
        o => panic!("{o:?}"),
    }
    match outcome("implies_fires") {
        Outcome::Failed(m) => {
            assert!(m.contains("assert_implies \"too_tight\": consequent did not follow within 1 cycles"), "{m}")
        }
        o => panic!("{o:?}"),
    }
    match outcome("x_detected") {
        Outcome::Failed(m) => assert!(m.contains("outputs: top.xsig is 4'bxxxx at"), "{m}"),
        o => panic!("{o:?}"),
    }
    assert_eq!(outcome("x_cleared"), Outcome::Passed);
}

struct Accumulator {
    sum: u32,
}

impl Model<u8, u32> for Accumulator {
    fn step(&mut self, input: &u8) -> Option<u32> {
        self.sum += *input as u32;
        if *input == 0 {
            None
        } else {
            Some(self.sum)
        }
    }
    fn reset(&mut self) {
        self.sum = 0;
    }
}

#[test]
fn model_scoreboard() {
    let mut sb = ModelScoreboard::new("acc", Accumulator { sum: 0 });
    sb.drive(3);
    sb.drive(0);
    sb.drive(4);
    assert_eq!(sb.driven(), 3);
    sb.observe(3);
    sb.observe(7);
    sb.finish().unwrap();
    sb.observe(99);
    assert!(sb.finish().is_err());
    let mut sb2 = ModelScoreboard::new("closure", |x: &u8| Some(*x as u16 * 2));
    sb2.drive(5);
    sb2.observe(10);
    sb2.finish().unwrap();
    sb2.reset();
    let shared = sb2.scoreboard();
    sb2.drive(1);
    shared.observe(2);
    sb2.finish().unwrap();
}

#[test]
fn traces_against_golden_files() {
    let dir = std::env::temp_dir().join(format!("rivet-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("RIVET_GOLDEN_DIR", &dir);
    std::env::remove_var("RIVET_UPDATE_GOLDEN");
    let make = |v: u64| {
        let dir = dir.clone();
        run_test(design(), move |dut| async move {
            let clk = dut.signal("clk")?;
            let mut t = Trace::new("events");
            t.record("start");
            clk.rising_edge().await;
            read_only().await;
            t.record(format!("count={} v={v}", dut.signal("count")?.get_u64_lossy()));
            Timer::new(7.ns()).await;
            t.record("end");
            assert_eq!(t.len(), 3);
            let text = t.text();
            assert!(text.contains("0ns start"), "{text}");
            let r = t.check_golden();
            let _ = dir;
            r
        })
    };
    // No golden yet: fails with instructions and writes the actual.
    let e = make(1).unwrap_err().to_string();
    assert!(e.contains("no golden file"), "{e}");
    assert!(dir.join("no_test__events.trace.actual").exists() || dir.read_dir().unwrap().count() > 0);
    // Accept, then compare: identical passes, a change fails with a diff.
    std::env::set_var("RIVET_UPDATE_GOLDEN", "1");
    make(1).unwrap();
    std::env::remove_var("RIVET_UPDATE_GOLDEN");
    make(1).unwrap();
    let e = make(2).unwrap_err().to_string();
    assert!(e.contains("differs from"), "{e}");
    assert!(e.contains("-      2") && e.contains("v=1") && e.contains("+      2") && e.contains("v=2"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
    std::env::remove_var("RIVET_GOLDEN_DIR");
    let mut u = Trace::unstamped("plain");
    u.record(Logic::One);
    assert_eq!(u.text(), "1\n");
    assert!(!u.is_empty());
}
