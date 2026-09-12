//! JSON log format and per-test log files, end to end through the runner.
//! Env vars are read when the logger is installed, so this binary sets
//! them before the first simulation.

use rivet_core::test::{boxed, TestDesc};
use rivet_core::triggers::Timer;
use rivet_core::{inventory, Module};
use rivet_mock::Design;

inventory::submit! {
    TestDesc { name: "logs_something", module: "logmod", run: |_dut: Module| boxed(async {
        Timer::steps(3).await;
        log::info!("hello from the test with a \"quote\"");
        log::warn!("and a warning");
        Ok(())
    }), timeout: || None, skip: false, expect_fail: false, stage: 0, wall_timeout: None, param_sets: &[] }
}

#[test]
fn json_format_and_per_test_files() {
    let dir = std::env::temp_dir().join(format!("rivet-logs-{}", std::process::id()));
    std::env::set_var("RIVET_LOG_FORMAT", "json");
    std::env::set_var("RIVET_LOG_DIR", &dir);
    let mut d = Design::new("top").precision(-9);
    let x = d.logic("x", 4);
    d.init(x, rivet_core::LogicVec::from_u64(4, 0));
    let results = rivet_mock::run_regression(d, rivet_core::test::all_tests(), Some("logs_something"));
    assert_eq!(results.len(), 1);
    let text = std::fs::read_to_string(dir.join("logmod__logs_something.log")).unwrap();
    let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let hello = lines.iter().find(|l| l["msg"].as_str().unwrap_or("").starts_with("hello")).expect("hello line");
    assert_eq!(hello["level"], "INFO");
    assert_eq!(hello["test"], "logmod::logs_something");
    assert_eq!(hello["t"], 3);
    assert_eq!(hello["time"], "3ns");
    assert_eq!(hello["msg"], "hello from the test with a \"quote\"");
    assert!(lines.iter().any(|l| l["level"] == "WARN" && l["msg"] == "and a warning"));
    // The runner's own "running ..." line is inside the test's file too.
    assert!(lines.iter().any(|l| l["msg"].as_str().unwrap_or("").starts_with("running logmod::logs_something")));
    std::fs::remove_dir_all(&dir).unwrap();
}
