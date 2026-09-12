//! Hang diagnostics: the task dump, timeouts that include it, the
//! wall-clock limit, and the watchdog.

use rivet_core::test::{boxed, Outcome, TestDesc};
use rivet_core::triggers::{read_only, yield_now, Timer};
use rivet_core::{inventory, runtime, spawn_named, Event, LogicVec, Module, Queue, TimeExt};
use rivet_mock::{run_test, Design};

fn clocked() -> Design {
    let mut d = Design::new("top").precision(-9);
    let clk = d.logic("clk", 1);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.clock(clk, 5, 0);
    d
}

#[test]
fn dump_lists_every_task_and_its_trigger() {
    run_test(clocked(), |dut| async move {
        let clk = dut.signal("clk")?;
        let q: Queue<u8> = Queue::bounded(1);
        let ev = Event::new();
        let q2 = q.clone();
        let ev2 = ev.clone();
        let _t1 = spawn_named("edge_waiter", async move {
            clk.rising_edge().await;
        });
        let _t2 = spawn_named("timer_waiter", async {
            Timer::new(1.us()).await;
        });
        let _t3 = spawn_named("ro_waiter", async {
            read_only().await;
        });
        let _t4 = spawn_named("queue_getter", async move {
            let _ = q2.get().await;
        });
        let _t5 = spawn_named("event_waiter", async move {
            ev2.wait().await;
        });
        let h = spawn_named("sleeper", async {
            Timer::new(2.us()).await;
        });
        let _t6 = spawn_named("joiner", async move {
            let _ = h.await;
        });
        yield_now().await;
        yield_now().await;
        let dump = runtime::dump_tasks();
        eprintln!("{dump}");
        assert!(dump.contains("edge_waiter"), "{dump}");
        assert!(dump.contains("RisingEdge(top.clk)"), "{dump}");
        assert!(dump.contains("timer_waiter") && dump.contains("Timer due at 1000ns (+1000ns)"), "{dump}");
        assert!(dump.contains("ro_waiter") && dump.contains("waiting for ReadOnly"), "{dump}");
        assert!(dump.contains("queue_getter") && dump.contains("waiting for Queue"), "{dump}");
        assert!(dump.contains("event_waiter") && dump.contains("waiting for Event"), "{dump}");
        assert!(dump.contains("joiner") && dump.contains("another task (join)"), "{dump}");
        assert!(dump.contains("test") && dump.contains("running"), "the current task is marked running: {dump}");
        assert!(dump.starts_with("9 live task(s) at 0ns"), "{dump}");
        Ok(())
    })
    .unwrap();
}

inventory::submit! {
    TestDesc { name: "hangs_in_sim_time", module: "diag", run: |dut: Module| boxed(async move {
        let clk = dut.signal("clk")?;
        let _mon = spawn_named("stuck_monitor", async { Timer::new(1.ms()).await; });
        loop { clk.rising_edge().await; }
    }), timeout: || Some(100.ns()), skip: false, expect_fail: false, stage: 0, wall_timeout: None, param_sets: &[] }
}

inventory::submit! {
    TestDesc { name: "hangs_in_wall_time", module: "diag", run: |_dut: Module| boxed(async move {
        loop { Timer::steps(1).await; }
    }), timeout: || None, skip: false, expect_fail: false, stage: 0, wall_timeout: Some(0.2), param_sets: &[] }
}

inventory::submit! {
    TestDesc { name: "finishes_in_time", module: "diag", run: |_dut: Module| boxed(async move {
        Timer::steps(10).await;
        Ok(())
    }), timeout: || Some(1.us()), skip: false, expect_fail: false, stage: 0, wall_timeout: Some(30.0), param_sets: &[] }
}

#[test]
fn timeouts_carry_the_task_dump() {
    let results = rivet_mock::run_regression(clocked(), rivet_core::test::all_tests(), Some("diag::"));
    let by_name = |n: &str| results.iter().find(|r| r.name == n).unwrap().outcome.clone();
    match by_name("hangs_in_sim_time") {
        Outcome::Failed(msg) => {
            assert!(msg.starts_with("timed out after 100ns of simulated time"), "{msg}");
            assert!(msg.contains("stuck_monitor"), "dump names the other task: {msg}");
            assert!(msg.contains("RisingEdge(top.clk)"), "{msg}");
            assert!(msg.contains("Timer due at"), "{msg}");
        }
        o => panic!("expected failure, got {o:?}"),
    }
    match by_name("hangs_in_wall_time") {
        Outcome::Failed(msg) => {
            assert!(msg.starts_with("wall-clock limit of 0.2s exceeded"), "{msg}");
            assert!(msg.contains("live task(s)"), "{msg}");
        }
        o => panic!("expected failure, got {o:?}"),
    }
    assert_eq!(by_name("finishes_in_time"), Outcome::Passed);
    // The limit is per test: the passing test after the hung one is unaffected.
    assert!(rivet_core::test::wall_timeout_from_env().is_none() || std::env::var("RIVET_WALL_TIMEOUT").is_ok());
}

/// The watchdog kills a process whose simulator never returns control. Run
/// as a child so the abort does not take the test runner with it.
#[test]
fn watchdog_aborts_a_stuck_process() {
    if std::env::var("RIVET_WATCHDOG_CHILD").is_ok() {
        rivet_core::test::start_watchdog();
        rivet_core::test::watchdog_arm_for_tests(0.1, "child");
        std::thread::sleep(std::time::Duration::from_secs(30));
        std::process::exit(0);
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "watchdog_aborts_a_stuck_process", "--nocapture"])
        .env("RIVET_WATCHDOG_CHILD", "1")
        .env("RIVET_WATCHDOG_GRACE", "0.2")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("watchdog: test child exceeded its wall-clock limit"), "{err}");
}
