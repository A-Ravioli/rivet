//! The regression runner itself (ordering, skip, expect_fail, timeouts,
//! panics, results.xml) exercised through the mock simulator with tests
//! registered the way `#[rivet::test]` registers them.

use rivet_core::test::{all_tests, boxed, summarize_with, write_results_xml_with, Outcome, TestDesc};
use rivet_core::triggers::Timer;
use rivet_core::{inventory, Module, TimeExt};
use rivet_mock::Design;

inventory::submit! {
    TestDesc { name: "b_passes", module: "suite", run: |_dut: Module| boxed(async { Timer::steps(3).await; Ok(()) }), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "a_fails", module: "suite", run: |_| boxed(async { Err(rivet_core::Error::Msg("nope".into())) }), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "c_expected_failure", module: "suite", run: |_| boxed(async { rivet_core::bail!("meant to") }), expect_fail: true, stage: 0, ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "d_unexpected_pass", module: "suite", run: |_| boxed(async { Ok(()) }), expect_fail: true, stage: 0, ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "e_skipped", module: "suite", run: |_| boxed(async { panic!("never runs") }), skip: true, expect_fail: false, stage: 0, ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "f_times_out", module: "suite", run: |_| boxed(async { Timer::new(1.ms()).await; Ok(()) }), timeout: || Some(20.ns()), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "g_panics", module: "suite", run: |_| boxed(async { Timer::steps(1).await; panic!("kaboom") }), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "h_child_panics", module: "suite", run: |_| boxed(async {
        let _h = rivet_core::spawn(async { Timer::steps(1).await; panic!("child kaboom") });
        Timer::steps(5).await;
        Ok(())
    }), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "z_first_by_stage", module: "suite", run: |dut: Module| boxed(async move {
        // Runs first: nothing has been written yet.
        assert_eq!(dut.signal("x").unwrap().get_u64_lossy(), 0);
        dut.signal("x").unwrap().set_now(1);
        Ok(())
    }), stage: -1, ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "bad_timeout", module: "suite", run: |_| boxed(async { Ok(()) }), timeout: || Some(1.5.steps()), stage: 1, ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "other_module", module: "elsewhere", run: |_| boxed(async { Ok(()) }), ..TestDesc::DEFAULT }
}

fn design() -> Design {
    let mut d = Design::new("top").precision(-9);
    let x = d.logic("x", 4);
    d.init(x, rivet_core::LogicVec::from_u64(4, 0));
    d
}

#[test]
fn full_regression_outcomes_and_order() {
    // Scoped to this test's own modules with a regular expression, so
    // registrations added elsewhere in the file do not change the list.
    let results = rivet_mock::run_regression(design(), all_tests(), Some("^(suite|elsewhere|sets)::"));
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "z_first_by_stage",
            "other_module",
            "any_set",
            "a_fails",
            "b_passes",
            "c_expected_failure",
            "d_unexpected_pass",
            "e_skipped",
            "f_times_out",
            "g_panics",
            "h_child_panics",
            "bad_timeout"
        ],
        "stage, then module, then name"
    );
    let outcome = |n: &str| &results.iter().find(|r| r.name == n).unwrap().outcome;
    assert_eq!(*outcome("b_passes"), Outcome::Passed);
    assert_eq!(*outcome("z_first_by_stage"), Outcome::Passed);
    assert_eq!(*outcome("other_module"), Outcome::Passed);
    assert!(matches!(outcome("a_fails"), Outcome::Failed(m) if m == "nope"));
    assert_eq!(*outcome("c_expected_failure"), Outcome::Passed, "expected failure counts as pass");
    assert!(matches!(outcome("d_unexpected_pass"), Outcome::Failed(m) if m.contains("expected to fail")));
    assert_eq!(*outcome("e_skipped"), Outcome::Skipped);
    assert!(matches!(outcome("f_times_out"), Outcome::Failed(m) if m.contains("timed out after 20ns")));
    assert!(matches!(outcome("g_panics"), Outcome::Failed(m) if m.contains("kaboom")));
    assert!(matches!(outcome("h_child_panics"), Outcome::Failed(m) if m.contains("child kaboom")));
    assert!(matches!(outcome("bad_timeout"), Outcome::Failed(m) if m.contains("invalid timeout")));
    // Sim time accounting.
    let b = results.iter().find(|r| r.name == "b_passes").unwrap();
    assert_eq!(b.sim_time_steps, 3);
    let f = results.iter().find(|r| r.name == "f_times_out").unwrap();
    assert_eq!(f.sim_time_steps, 20);
    assert_eq!(summarize_with(&results, -9), 6, "failed count");
}

#[test]
fn filter_selects_subset() {
    let results = rivet_mock::run_regression(design(), all_tests(), Some("b_passes, elsewhere::other"));
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["other_module", "b_passes"]);
    assert!(results.iter().all(|r| r.outcome == Outcome::Passed));
}

#[test]
fn results_xml_round_trip() {
    let results = rivet_mock::run_regression(design(), all_tests(), Some("b_passes,a_fails,e_skipped"));
    let dir = std::env::temp_dir().join(format!("rivet-regression-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("results.xml");
    write_results_xml_with(&path, &results, "mock", -9).unwrap();
    let x = std::fs::read_to_string(&path).unwrap();
    assert!(x.starts_with("<?xml"));
    assert!(
        x.contains(r#"<testsuite name="suite" package="suite" tests="3" failures="1" errors="0" skipped="1">"#),
        "{x}"
    );
    assert!(x.contains(r#"<testcase name="a_fails" classname="suite""#));
    assert!(x.contains(r#"<failure message="nope" />"#));
    assert!(x.contains("<skipped />"));
    assert!(x.contains(r#"<property name="cocotb" value="True" />"#));
    assert!(x.contains(r#"<property name="simulator" value="mock" />"#));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hierarchy_dump_is_valid_json() {
    let mut d = Design::new("top");
    let sub = d.module(d.root(), "u_sub");
    d.logic_in(sub, "leaf", 5);
    d.logic("clk", 1);
    d.param("P", 8, 3);
    let dir = std::env::temp_dir().join(format!("rivet-dump-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("h.json");
    let p2 = path.clone();
    rivet_mock::run_test(d, move |dut| async move {
        rivet_core::test::dump_hierarchy(dut, &p2).unwrap();
        Ok(())
    })
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["name"], "top");
    assert_eq!(v["kind"], "Module");
    let children = v["children"].as_array().unwrap();
    assert_eq!(children.len(), 3);
    let sub = children.iter().find(|c| c["name"] == "u_sub").unwrap();
    assert_eq!(sub["children"][0]["name"], "leaf");
    assert_eq!(sub["children"][0]["width"], 5);
    let p = children.iter().find(|c| c["name"] == "P").unwrap();
    assert_eq!(p["is_const"], true);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn simulator_ending_early_fails_the_running_test() {
    let mut d = Design::new("top").precision(-9);
    d.logic("x", 1);
    d.at(5, |s| s.finish());
    let r = rivet_mock::run_test(d, |_| async move {
        Timer::new(100.ns()).await;
        Ok(())
    });
    let msg = r.unwrap_err().to_string();
    assert!(msg.contains("ended before"), "{msg}");
}

inventory::submit! {
    TestDesc { name: "only_w16", module: "sets", run: |_| boxed(async { Ok(()) }), param_sets: &["w16"], ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "any_set", module: "sets", run: |_| boxed(async { Ok(()) }), ..TestDesc::DEFAULT }
}

#[test]
fn param_sets_select_tests() {
    use rivet_core::test::selected;
    let tests = all_tests();
    let only = tests.iter().find(|t| t.name == "only_w16").unwrap();
    let any = tests.iter().find(|t| t.name == "any_set").unwrap();
    std::env::remove_var("RIVET_PARAM_SET");
    assert!(!selected(only, None), "set-specific test skipped without a set");
    assert!(selected(any, None));
    std::env::set_var("RIVET_PARAM_SET", "w8");
    assert!(!selected(only, None));
    std::env::set_var("RIVET_PARAM_SET", "w16");
    assert!(selected(only, None));
    assert!(selected(any, Some("any")));
    assert_eq!(rivet_core::test::param_set().as_deref(), Some("w16"));
    std::env::remove_var("RIVET_PARAM_SET");
}

// ---------------------------------------------------------------------------
// Parity with cocotb's regression manager: regular-expression filters, a
// reproducible shuffle, failure-message matching, expected timeouts, an
// early pass from any task, and the source location in results.xml.

inventory::submit! {
    TestDesc { name: "msg_matches", module: "parity", run: |_| boxed(async {
        rivet_core::bail!("counter overflowed at 42")
    }), expect_fail: true, expect_fail_msg: Some("overflowed"), file: file!(), line: line!(), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "msg_does_not_match", module: "parity", run: |_| boxed(async {
        rivet_core::bail!("something else entirely")
    }), expect_fail: true, expect_fail_msg: Some("overflowed"), ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "times_out_on_purpose", module: "parity", run: |_| boxed(async {
        Timer::new(1.ms()).await;
        Ok(())
    }), timeout: || Some(20.ns()), expect_timeout: true, ..TestDesc::DEFAULT }
}
inventory::submit! {
    TestDesc { name: "finished_by_a_child", module: "parity", run: |_| boxed(async {
        // A monitor decides the test is done; the main task would otherwise
        // wait for a lot longer.
        let _m = rivet_core::spawn(async {
            Timer::steps(2).await;
            rivet_core::runtime::finish_test();
        });
        Timer::new(1.ms()).await;
        Ok(())
    }), timeout: || Some(500.us()), ..TestDesc::DEFAULT }
}

#[test]
fn message_matching_expected_timeouts_and_early_finish() {
    let results = rivet_mock::run_regression(design(), all_tests(), Some("parity::"));
    let by = |n: &str| results.iter().find(|r| r.name == n).unwrap().outcome.clone();
    assert_eq!(by("msg_matches"), Outcome::Passed, "the message contains what was expected");
    match by("msg_does_not_match") {
        Outcome::Failed(m) => assert!(m.contains("does not contain"), "{m}"),
        o => panic!("a failure for the wrong reason must not pass: {o:?}"),
    }
    assert_eq!(by("times_out_on_purpose"), Outcome::Passed);
    // finish_test() ended the test early, well before its 500us timeout.
    assert_eq!(by("finished_by_a_child"), Outcome::Passed);
    let t = results.iter().find(|r| r.name == "finished_by_a_child").unwrap();
    assert!(t.sim_time_steps < 100, "ended at the child's call, not the timeout: {}", t.sim_time_steps);
}

#[test]
fn filters_are_regular_expressions() {
    use rivet_core::test::pattern_matches;
    assert!(pattern_matches("counts$", "example::counter_counts"));
    assert!(pattern_matches("^example::", "example::counter_counts"));
    assert!(pattern_matches("count(er|ing)", "example::counter_counts"));
    assert!(!pattern_matches("^counts", "example::counter_counts"));
    // An invalid pattern is a plain substring, so a bracket in a parametrised
    // test name still selects it.
    assert!(pattern_matches("bursts[16]", "bus::axi_mem_bursts[16]"));
}

#[test]
fn shuffle_is_reproducible_and_respects_stages() {
    // Same seed, same order; different seed, different order (with enough
    // tests for that to be overwhelmingly likely).
    let order = |seed: u64| {
        rivet_core::random::set_base_seed(seed);
        std::env::set_var("RIVET_SHUFFLE", "1");
        let v = all_tests();
        std::env::remove_var("RIVET_SHUFFLE");
        let stages: Vec<i32> = v.iter().map(|t| t.stage).collect();
        let mut sorted = stages.clone();
        sorted.sort();
        assert_eq!(stages, sorted, "shuffling never moves a test out of its stage");
        v.iter().map(|t| format!("{}::{}", t.module, t.name)).collect::<Vec<_>>()
    };
    let a = order(1234);
    let b = order(1234);
    let c = order(9999);
    assert_eq!(a, b, "the same seed replays the same order");
    assert_ne!(a, c, "a different seed shuffles differently");
}

#[test]
fn results_xml_carries_the_source_location() {
    let results = rivet_mock::run_regression(design(), all_tests(), Some("parity::msg_matches"));
    let dir = std::env::temp_dir().join(format!("rivet-loc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("results.xml");
    write_results_xml_with(&path, &results, "mock", -9).unwrap();
    let xml = std::fs::read_to_string(&path).unwrap();
    // The registration above is in this file, so the location points here.
    assert!(xml.contains(r#"file="crates/rivet-mock/tests/regression.rs""#), "{xml}");
    assert!(!xml.contains(r#"lineno="0""#), "{xml}");
    let _ = std::fs::remove_dir_all(&dir);
}
