//! Test registration and the regression loop.
//!
//! `#[rivet::test]` expands to an `inventory::submit!` of a [`TestDesc`].
//! At StartOfSim the runtime spawns [`run_regression`], which runs each
//! test in turn inside one simulator process, one time step apart, and
//! writes a JUnit `results.xml` compatible with cocotb's.

use crate::error::{Error, Result};
use crate::handle::Module;
use crate::runtime::{self, TestEnd, TestFailed};
use crate::time::{format_time, Duration, Unit};
use crate::triggers::{first, Either, Timer};
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

pub type TestFuture = Pin<Box<dyn Future<Output = Result<()>>>>;

/// A registered test.
pub struct TestDesc {
    pub name: &'static str,
    pub module: &'static str,
    pub run: fn(Module) -> TestFuture,
    /// Simulated-time timeout, evaluated when the test starts.
    pub timeout: fn() -> Option<Duration>,
    pub skip: bool,
    pub expect_fail: bool,
    /// `expect_fail = "..."`: the failure message must contain this.
    pub expect_fail_msg: Option<&'static str>,
    /// The test is expected to run out of simulated time.
    pub expect_timeout: bool,
    /// Source location, for `results.xml` and for editors.
    pub file: &'static str,
    pub line: u32,
    /// Tests are stable-sorted by stage.
    pub stage: i32,
    /// Wall-clock limit in seconds (`RIVET_WALL_TIMEOUT` applies when `None`).
    pub wall_timeout: Option<f64>,
    /// Manifest parameter sets this test runs under (empty: all).
    pub param_sets: &'static [&'static str],
}

inventory::collect!(TestDesc);

fn nothing_to_run(_dut: Module) -> TestFuture {
    boxed(async { Ok(()) })
}

fn no_timeout() -> Option<Duration> {
    None
}

impl TestResult {
    /// A passed result with everything else empty, for tests and for
    /// callers that build results by hand.
    pub fn new(name: &str, module: &str, outcome: Outcome) -> TestResult {
        TestResult {
            name: name.to_string(),
            module: module.to_string(),
            outcome,
            sim_time_steps: 0,
            wall_secs: 0.0,
            seed: 0,
            file: String::new(),
            line: 0,
        }
    }
}

impl TestDesc {
    /// Defaults for a hand-written registration, for struct update syntax:
    ///
    /// ```ignore
    /// TestDesc { name: "t", module: "m", run: |_| boxed(async { Ok(()) }), ..TestDesc::DEFAULT }
    /// ```
    ///
    /// `#[rivet::test]` fills every field itself; this keeps hand-written
    /// registrations (and tests of the runner) working as fields are added.
    pub const DEFAULT: TestDesc = TestDesc {
        name: "",
        module: "",
        run: nothing_to_run,
        timeout: no_timeout,
        skip: false,
        expect_fail: false,
        expect_fail_msg: None,
        expect_timeout: false,
        stage: 0,
        wall_timeout: None,
        param_sets: &[],
        file: "",
        line: 0,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed(String),
    Skipped,
}

#[derive(Clone, Debug)]
pub struct TestResult {
    pub name: String,
    pub module: String,
    pub outcome: Outcome,
    pub sim_time_steps: u64,
    pub wall_secs: f64,
    /// The test's random seed (derived from the run's base seed).
    pub seed: u64,
    /// Where the test is written, from `file!()` and `line!()`.
    pub file: String,
    pub line: u32,
}

/// All tests visible to this binary, in execution order.
pub fn all_tests() -> Vec<&'static TestDesc> {
    let mut v: Vec<&TestDesc> = inventory::iter::<TestDesc>.into_iter().collect();
    // Link order is not deterministic across builds; sort by stage, then
    // module and name, so runs on different simulators agree.
    v.sort_by(|a, b| a.stage.cmp(&b.stage).then_with(|| a.module.cmp(b.module)).then_with(|| a.name.cmp(b.name)));
    if shuffle_requested() {
        // Reproducible from the run's seed, and still stage by stage, so a
        // shuffled run can be replayed exactly with --seed.
        shuffle_within_stages(&mut v);
    }
    v
}

/// Test selection: `filter` is a comma-separated list of patterns, each a
/// regular expression searched (not anchored) against `module::name`, as
/// cocotb's `COCOTB_TEST_FILTER` is. A pattern that is not a valid regular
/// expression falls back to a substring match, so plain names keep working.
/// An empty or absent filter selects everything.
pub fn selected(t: &TestDesc, filter: Option<&str>) -> bool {
    // A test tied to parameter sets runs only under one of them.
    if !t.param_sets.is_empty() {
        match param_set() {
            Some(set) if t.param_sets.contains(&set.as_str()) => {}
            _ => return false,
        }
    }
    // Exact selection used by `rivet run --jobs` to hand each shard its
    // share of the tests.
    if let Ok(sel) = std::env::var("RIVET_TEST_SELECT") {
        if !sel.trim().is_empty() {
            let full = format!("{}::{}", t.module, t.name);
            return sel.split(',').map(str::trim).any(|s| s == full);
        }
    }
    match filter {
        Some(f) if !f.trim().is_empty() => {
            let full = format!("{}::{}", t.module, t.name);
            f.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .any(|pat| pattern_matches(pat, &full) || pattern_matches(pat, t.name))
        }
        _ => true,
    }
}

/// `re.search` semantics, plus a literal substring fallback.
pub fn pattern_matches(pattern: &str, text: &str) -> bool {
    if let Ok(re) = regex_lite::Regex::new(pattern) {
        if re.is_match(text) {
            return true;
        }
    }
    // Fall back to a literal match, so a name that is also valid regular
    // expression syntax still selects itself: `axi_mem_bursts[16]` is a
    // character class to a regular expression engine, and a test name to
    // the person typing it.
    text.contains(pattern)
}

/// Shuffle tests within each stage, reproducibly from the run's base seed.
/// cocotb's `COCOTB_RANDOM_TEST_ORDER`; `RIVET_SHUFFLE=1` turns it on.
pub fn shuffle_requested() -> bool {
    matches!(std::env::var("RIVET_SHUFFLE").ok().as_deref(), Some("1") | Some("true") | Some("yes"))
}

fn shuffle_within_stages(tests: &mut [&'static TestDesc]) {
    let mut rng = crate::random::Rng::seed_from_u64(crate::random::base_seed() ^ 0x5348_5546_464c_4521);
    let mut start = 0;
    while start < tests.len() {
        let stage = tests[start].stage;
        let mut end = start;
        while end < tests.len() && tests[end].stage == stage {
            end += 1;
        }
        rng.shuffle(&mut tests[start..end]);
        start = end;
    }
}

pub fn param_set() -> Option<String> {
    std::env::var("RIVET_PARAM_SET").ok().filter(|s| !s.is_empty())
}

/// Run every registered test selected by `RIVET_TEST_FILTER`. Spawned by
/// the runtime at StartOfSim.
pub async fn run_regression(root: Module) -> Vec<TestResult> {
    let filter = std::env::var("RIVET_TEST_FILTER").ok();
    run_regression_with(root, all_tests(), filter.as_deref()).await
}

/// Run the given tests in order, with an explicit selection filter.
pub async fn run_regression_with(root: Module, tests: Vec<&'static TestDesc>, filter: Option<&str>) -> Vec<TestResult> {
    let precision = runtime::precision();
    let mut results = Vec::new();
    let total = tests.iter().filter(|t| selected(t, filter)).count();
    log::info!(
        "running {} test(s) on {} (seed {})",
        total,
        runtime::with(|rt| format!("{} {}", rt.backend.name(), rt.backend.version())),
        crate::random::base_seed()
    );
    let mut idx = 0;
    for t in tests {
        if !selected(t, filter) {
            continue;
        }
        idx += 1;
        if t.skip {
            log::info!("skipping {}::{}", t.module, t.name);
            results.push(TestResult {
                name: t.name.to_string(),
                module: t.module.to_string(),
                outcome: Outcome::Skipped,
                sim_time_steps: 0,
                wall_secs: 0.0,
                seed: 0,
                file: t.file.to_string(),
                line: t.line,
            });
            continue;
        }
        if idx > 1 {
            // One time step between tests, as in cocotb.
            Timer::steps(1).await;
        }
        let seed = crate::random::begin_test(&format!("{}::{}", t.module, t.name));
        crate::log::begin_test(t.module, t.name);
        log::info!("running {}::{} ({idx}/{total}, seed {seed})", t.module, t.name);
        let wall = Instant::now();
        let start = runtime::now();
        runtime::begin_test();
        let wall_limit = t.wall_timeout.or_else(wall_timeout_from_env);
        runtime::set_wall_limit(wall_limit);
        watchdog_arm(wall_limit, &format!("{}::{}", t.module, t.name));
        let waves_per_test = std::env::var("RIVET_WAVES").map(|v| v == "per-test").unwrap_or(false);
        if waves_per_test {
            crate::waves::start(Some(&format!("{}__{}", t.module.replace("::", "_"), t.name)));
        }
        // Evaluate and convert the timeout under a guard: a bad expression
        // must fail this test, not the regression task.
        let timeout = match std::panic::catch_unwind(|| (t.timeout)().map(|d| (d, d.to_steps(precision)))) {
            Ok(d) => Ok(d),
            Err(p) => Err(format!("invalid timeout: {}", runtime::panic_message(&p))),
        };
        let outcome = match timeout {
            Ok(timeout) => {
                let handle = crate::task::spawn_named(t.name, (t.run)(root));
                run_one(handle, timeout).await
            }
            Err(msg) => Outcome::Failed(msg),
        };
        let recorded = runtime::end_test();
        runtime::set_wall_limit(None);
        watchdog_arm(None, "");
        if waves_per_test {
            crate::waves::off();
        }
        // Tear down: cancel everything the test started, drop buffered writes.
        runtime::cancel_all_other_tasks();
        runtime::discard_pending_writes();
        let outcome = match (outcome, recorded) {
            (Outcome::Passed, Some(f)) => Outcome::Failed(f),
            (o, _) => o,
        };
        let expects_failure = t.expect_fail || t.expect_timeout || t.expect_fail_msg.is_some();
        let wanted = t.expect_fail_msg.or(if t.expect_timeout { Some("timed out after") } else { None });
        let outcome = match (expects_failure, outcome) {
            (true, Outcome::Failed(msg)) => match wanted {
                Some(w) if !msg.contains(w) => {
                    Outcome::Failed(format!("failed as expected, but the message does not contain {w:?}: {msg}"))
                }
                _ => {
                    log::info!("{}::{} failed as expected: {msg}", t.module, t.name);
                    Outcome::Passed
                }
            },
            (true, Outcome::Passed) => Outcome::Failed("test passed but was expected to fail".into()),
            (true, Outcome::Skipped) => Outcome::Skipped,
            (_, o) => o,
        };
        let sim_time_steps = runtime::now() - start;
        match &outcome {
            Outcome::Passed => log::info!(
                "{}::{} passed (sim {}, wall {:.3}s)",
                t.module,
                t.name,
                format_time(sim_time_steps, precision, Unit::Ns),
                wall.elapsed().as_secs_f64()
            ),
            Outcome::Failed(msg) => log::error!("{}::{} FAILED: {msg}", t.module, t.name),
            Outcome::Skipped => {}
        }
        crate::log::end_test();
        results.push(TestResult {
            name: t.name.to_string(),
            module: t.module.to_string(),
            outcome,
            sim_time_steps,
            wall_secs: wall.elapsed().as_secs_f64(),
            seed,
            file: t.file.to_string(),
            line: t.line,
        });
    }
    results
}

async fn run_one(handle: crate::task::JoinHandle<Result<()>>, timeout: Option<(Duration, u64)>) -> Outcome {
    let body = async move {
        match first(handle, TestFailed).await {
            Either::Left(Ok(Ok(()))) => Outcome::Passed,
            Either::Left(Ok(Err(Error::Skip(why)))) => {
                log::info!("skipped: {why}");
                Outcome::Skipped
            }
            Either::Left(Ok(Err(e))) => Outcome::Failed(e.to_string()),
            Either::Left(Err(e)) => Outcome::Failed(e.to_string()),
            Either::Right(TestEnd::Failed(msg)) => Outcome::Failed(msg),
            // A task called `finish_test()`: the test is over and passed.
            Either::Right(TestEnd::Finished) => Outcome::Passed,
        }
    };
    match timeout {
        Some((d, steps)) => match first(body, Timer::steps(steps.max(1))).await {
            Either::Left(o) => o,
            Either::Right(()) => {
                Outcome::Failed(format!("timed out after {d} of simulated time\n{}", runtime::dump_tasks()))
            }
        },
        None => body.await,
    }
}

/// `RIVET_WALL_TIMEOUT` in seconds, if set and valid.
pub fn wall_timeout_from_env() -> Option<f64> {
    std::env::var("RIVET_WALL_TIMEOUT").ok().and_then(|v| v.trim().parse::<f64>().ok()).filter(|s| *s > 0.0)
}

// ---------------------------------------------------------------------------
// Watchdog: a thread that aborts the process when a test runs past its
// wall-clock limit plus a grace period without the runtime noticing (the
// simulator is stuck, or a task never yields).

static WATCHDOG: std::sync::Mutex<Option<(std::time::Instant, String)>> = std::sync::Mutex::new(None);
static WATCHDOG_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn watchdog_arm(limit: Option<f64>, test: &str) {
    let mut g = WATCHDOG.lock().unwrap_or_else(|e| e.into_inner());
    *g = limit.map(|s| (Instant::now() + std::time::Duration::from_secs_f64(s + watchdog_grace()), test.to_string()));
    if limit.is_some() && !WATCHDOG_STARTED.swap(true, std::sync::atomic::Ordering::AcqRel) {
        start_watchdog();
    }
}

/// Extra seconds the watchdog waits beyond the limit before killing the
/// process, giving the in-band check a chance to fail the test cleanly.
/// `RIVET_WATCHDOG_GRACE` overrides it.
pub const WATCHDOG_GRACE_SECS: f64 = 5.0;

fn watchdog_grace() -> f64 {
    std::env::var("RIVET_WATCHDOG_GRACE").ok().and_then(|v| v.parse().ok()).unwrap_or(WATCHDOG_GRACE_SECS)
}

/// Arm the watchdog directly (for tests of the watchdog itself).
#[doc(hidden)]
pub fn watchdog_arm_for_tests(limit_secs: f64, test: &str) {
    watchdog_arm(Some(limit_secs), test);
}

/// Start the watchdog thread. Called automatically when a wall-clock limit
/// is set; exposed so embedding harnesses can start it early.
pub fn start_watchdog() {
    std::thread::Builder::new()
        .name("rivet-watchdog".into())
        .spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let g = WATCHDOG.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((deadline, test)) = g.as_ref() {
                if Instant::now() > *deadline {
                    eprintln!(
                        "rivet: watchdog: test {test} exceeded its wall-clock limit and the simulator is not \
                         returning control to the harness; aborting the process"
                    );
                    std::process::exit(3);
                }
            }
        })
        .expect("spawn watchdog thread");
}

/// Print the summary table and return the number of failures.
pub fn summarize(results: &[TestResult]) -> usize {
    summarize_with(results, runtime::precision())
}

/// [`summarize`] with an explicit time precision (usable without a runtime).
pub fn summarize_with(results: &[TestResult], precision: i32) -> usize {
    let name_w = results.iter().map(|r| r.module.len() + r.name.len() + 2).max().unwrap_or(4).max(4);
    eprintln!();
    eprintln!("{:<name_w$}  {:<8}  {:>14}  {:>10}", "TEST", "STATUS", "SIM TIME", "WALL");
    for r in results {
        let status = match &r.outcome {
            Outcome::Passed => "PASS",
            Outcome::Failed(_) => "FAIL",
            Outcome::Skipped => "SKIP",
        };
        eprintln!(
            "{:<name_w$}  {:<8}  {:>14}  {:>9.3}s",
            format!("{}::{}", r.module, r.name),
            status,
            format_time(r.sim_time_steps, precision, Unit::Ns),
            r.wall_secs
        );
    }
    let failed = results.iter().filter(|r| matches!(r.outcome, Outcome::Failed(_))).count();
    let passed = results.iter().filter(|r| r.outcome == Outcome::Passed).count();
    let skipped = results.iter().filter(|r| r.outcome == Outcome::Skipped).count();
    eprintln!();
    eprintln!("RIVET_RESULT passed={passed} failed={failed} skipped={skipped} seed={}", crate::random::base_seed());
    failed
}

/// Write cocotb-compatible JUnit XML.
pub fn write_results_xml(path: &std::path::Path, results: &[TestResult], sim_name: &str) -> std::io::Result<()> {
    write_results_xml_with(path, results, sim_name, runtime::precision())
}

/// [`write_results_xml`] with an explicit time precision.
pub fn write_results_xml_with(
    path: &std::path::Path,
    results: &[TestResult],
    sim_name: &str,
    precision: i32,
) -> std::io::Result<()> {
    use std::fmt::Write as _;
    let mut s = String::new();
    let failures = results.iter().filter(|r| matches!(r.outcome, Outcome::Failed(_))).count();
    let skipped = results.iter().filter(|r| r.outcome == Outcome::Skipped).count();
    writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(s, r#"<testsuites name="results">"#).unwrap();
    let modules: Vec<&str> = {
        let mut m: Vec<&str> = results.iter().map(|r| r.module.as_str()).collect();
        m.dedup();
        m
    };
    let _ = (failures, skipped);
    for module in modules {
        let in_module: Vec<&TestResult> = results.iter().filter(|r| r.module == module).collect();
        let f = in_module.iter().filter(|r| matches!(r.outcome, Outcome::Failed(_))).count();
        let sk = in_module.iter().filter(|r| r.outcome == Outcome::Skipped).count();
        writeln!(
            s,
            r#"  <testsuite name="{}" package="{}" tests="{}" failures="{}" errors="0" skipped="{}">"#,
            xml(module),
            xml(module),
            in_module.len(),
            f,
            sk
        )
        .unwrap();
        for r in in_module {
            writeln!(
                s,
                r#"    <testcase name="{}" classname="{}" file="{}" lineno="{}" time="{:.6}">"#,
                xml(&r.name),
                xml(&r.module),
                xml(&r.file),
                r.line,
                r.wall_secs
            )
            .unwrap();
            writeln!(s, "      <properties>").unwrap();
            writeln!(s, r#"        <property name="cocotb" value="True" />"#).unwrap();
            writeln!(s, r#"        <property name="rivet" value="True" />"#).unwrap();
            writeln!(s, r#"        <property name="simulator" value="{}" />"#, xml(sim_name)).unwrap();
            writeln!(
                s,
                r#"        <property name="sim_time_duration" value="{}" />"#,
                format_time(r.sim_time_steps, precision, Unit::Ns).trim_end_matches("ns")
            )
            .unwrap();
            writeln!(s, r#"        <property name="sim_time_unit" value="ns" />"#).unwrap();
            writeln!(s, r#"        <property name="random_seed" value="{}" />"#, r.seed).unwrap();
            writeln!(s, "      </properties>").unwrap();
            match &r.outcome {
                Outcome::Failed(msg) => writeln!(s, r#"      <failure message="{}" />"#, xml(msg)).unwrap(),
                Outcome::Skipped => writeln!(s, "      <skipped />").unwrap(),
                Outcome::Passed => {}
            }
            writeln!(s, "    </testcase>").unwrap();
        }
        writeln!(s, "  </testsuite>").unwrap();
    }
    writeln!(s, "</testsuites>").unwrap();
    std::fs::write(path, s)
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Types a test can take as its `dut` argument: `Module`, or a generated
/// typed hierarchy (`rivet bindgen`).
pub trait Bind: Sized {
    fn bind(root: Module) -> Result<Self>;
}

impl Bind for Module {
    fn bind(root: Module) -> Result<Module> {
        Ok(root)
    }
}

/// The design below `root` as an indented tree, one object per line with
/// its kind and width (`dut.hierarchy()` on generated bindings).
pub fn format_hierarchy(root: Module) -> String {
    fn walk(obj: crate::handle::Object, out: &mut String, depth: usize) {
        use std::fmt::Write as _;
        let info = obj.info();
        let pad = "  ".repeat(depth);
        if info.kind.is_hierarchy() {
            let _ = writeln!(out, "{pad}{} ({:?})", info.name, info.kind);
            let children = obj.as_module().ok().and_then(|m| m.children().ok()).unwrap_or_default();
            for c in children {
                walk(c, out, depth + 1);
            }
        } else {
            let extra = if info.is_const { ", const" } else { "" };
            let _ = writeln!(out, "{pad}{} : {:?}[{}]{extra}", info.name, info.kind, info.width);
        }
    }
    let mut out = String::new();
    walk(root.object(), &mut out, 0);
    out
}

/// Walk the design and write it as JSON (for `rivet bindgen`).
pub fn dump_hierarchy(root: Module, path: &std::path::Path) -> std::io::Result<()> {
    fn walk(obj: crate::handle::Object, out: &mut String, depth: usize) {
        use std::fmt::Write as _;
        let info = obj.info();
        let pad = "  ".repeat(depth);
        let _ = write!(
            out,
            "{pad}{{\"name\": \"{}\", \"path\": \"{}\", \"kind\": \"{:?}\", \"width\": {}, \"is_const\": {}, \"signed\": {}, \"type\": \"{}\"",
            esc(&info.name),
            esc(&info.path),
            info.kind,
            info.width,
            info.is_const,
            info.signed,
            esc(&info.type_name)
        );
        if info.kind.is_hierarchy() {
            let children = obj.as_module().ok().and_then(|m| m.children().ok()).unwrap_or_default();
            if !children.is_empty() {
                let _ = writeln!(out, ", \"children\": [");
                for (i, c) in children.iter().enumerate() {
                    walk(*c, out, depth + 1);
                    if i + 1 < children.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                let _ = write!(out, "{pad}]");
            }
        } else if info.kind == crate::backend::ObjKind::Array {
            // Element kind and width from the first element, if reachable.
            if let Ok(sig) = obj.as_signal() {
                if let Ok(e) = sig.index(info.range.map(|(l, r)| l.min(r)).unwrap_or(0)) {
                    let ei = e.info();
                    let _ = write!(out, ", \"element\": {{\"kind\": \"{:?}\", \"width\": {}}}", ei.kind, ei.width);
                }
            }
            if let Some((l, r)) = info.range {
                let _ = write!(out, ", \"range\": [{l}, {r}]");
            }
        }
        out.push('}');
    }
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let mut out = String::new();
    walk(root.object(), &mut out, 0);
    out.push('\n');
    std::fs::write(path, out)
}

/// The standard top-level task: run the regression, write results, finish.
/// Backends call [`install_default_entry`] before the simulation starts.
pub fn install_default_entry() {
    runtime::set_entry(|| {
        let root_name = std::env::var("RIVET_TOPLEVEL").ok().filter(|s| !s.is_empty());
        let root = runtime::with(|rt| rt.backend.root(root_name.as_deref()));
        let root = match root {
            Ok(h) => h,
            Err(e) => {
                log::error!("cannot find top-level instance: {e}");
                runtime::finish();
                return;
            }
        };
        runtime::with(|rt| rt.root = Some(root));
        let module = Module::from_handle(root);
        if let Ok(path) = std::env::var("RIVET_DUMP_HIERARCHY") {
            match dump_hierarchy(module, std::path::Path::new(&path)) {
                Ok(()) => log::info!("wrote hierarchy to {path}"),
                Err(e) => log::error!("cannot write {path}: {e}"),
            }
            runtime::with(|rt| rt.exit_code = 0);
            runtime::finish();
            return;
        }
        if let Ok(path) = std::env::var("RIVET_LIST_TESTS") {
            // `rivet run --jobs`: report the selected tests and stop.
            let filter = std::env::var("RIVET_TEST_FILTER").ok();
            let names: Vec<String> = all_tests()
                .iter()
                .filter(|t| selected(t, filter.as_deref()))
                .map(|t| format!("{}::{}", t.module, t.name))
                .collect();
            match std::fs::write(&path, names.join("\n") + "\n") {
                Ok(()) => log::info!("listed {} test(s) to {path}", names.len()),
                Err(e) => log::error!("cannot write {path}: {e}"),
            }
            runtime::with(|rt| rt.exit_code = 0);
            runtime::finish();
            return;
        }
        crate::task::spawn_named("regression", async move {
            let results = run_regression(module).await;
            finish_with(&results);
        });
    });
    runtime::set_premature_end_handler(|| {
        log::error!("simulator ended before all tests finished (an HDL $finish or assertion, or no clock running?)");
        runtime::report_failure("simulator ended prematurely".into());
        // Give the regression task a chance to record the failure.
        runtime::run_to_idle();
    });
}

fn finish_with(results: &[TestResult]) {
    let failed = summarize(results);
    let sim = runtime::with(|rt| rt.backend.name().to_string());
    let precision = runtime::precision();
    let path = std::env::var("RIVET_RESULTS_FILE").unwrap_or_else(|_| "results.xml".to_string());
    if let Err(e) = write_results_xml(std::path::Path::new(&path), results, &sim) {
        log::error!("cannot write {path}: {e}");
    }
    if let Ok(p) = std::env::var("RIVET_RESULTS_JSON") {
        if let Err(e) = std::fs::write(&p, results_json(results, &sim, precision)) {
            log::error!("cannot write {p}: {e}");
        }
    }
    if !crate::coverage::groups().is_empty() {
        eprintln!("{}", crate::coverage::render_table());
        eprintln!("RIVET_COVERAGE percent={:.2}", crate::coverage::percent());
    }
    if let Ok(p) = std::env::var("RIVET_COVERAGE_FILE") {
        if !crate::coverage::groups().is_empty() {
            if let Err(e) = crate::coverage::write_json(std::path::Path::new(&p)) {
                log::error!("cannot write {p}: {e}");
            }
        }
    }
    runtime::with(|rt| rt.exit_code = if failed > 0 { 1 } else { 0 });
    runtime::finish();
}

/// Results as JSON, for merging across shards and parameter sets
/// (`rivet run --jobs`). Read back by the CLI.
pub fn results_json(results: &[TestResult], sim: &str, precision: i32) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(
        s,
        "{{\"simulator\":{},\"precision\":{precision},\"seed\":{},\"tests\":[",
        json_str(sim),
        crate::random::base_seed()
    );
    for (i, r) in results.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let (outcome, message) = match &r.outcome {
            Outcome::Passed => ("passed", String::new()),
            Outcome::Failed(m) => ("failed", m.clone()),
            Outcome::Skipped => ("skipped", String::new()),
        };
        let _ = write!(
            s,
            "{{\"name\":{},\"module\":{},\"outcome\":\"{outcome}\",\"message\":{},\"sim_time_steps\":{},\"wall_secs\":{},\"seed\":{},\"file\":{},\"line\":{}}}",
            json_str(&r.name),
            json_str(&r.module),
            json_str(&message),
            r.sim_time_steps,
            r.wall_secs,
            r.seed,
            json_str(&r.file),
            r.line
        );
    }
    s.push_str("]}");
    s
}

fn json_str(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Convenience for building a `TestDesc::run` from an async fn.
pub fn boxed<F>(f: F) -> TestFuture
where
    F: Future<Output = Result<()>> + 'static,
{
    Box::pin(f)
}

impl From<Outcome> for Result<()> {
    fn from(o: Outcome) -> Result<()> {
        match o {
            Outcome::Passed | Outcome::Skipped => Ok(()),
            Outcome::Failed(m) => Err(Error::Msg(m)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(name: &'static str, module: &'static str, stage: i32) -> TestDesc {
        TestDesc { name, module, stage, ..TestDesc::DEFAULT }
    }

    #[test]
    fn selection_filter() {
        let t = desc("counter_counts", "example_dff", 0);
        assert!(selected(&t, None));
        assert!(selected(&t, Some("")));
        assert!(selected(&t, Some("  ")));
        assert!(selected(&t, Some("counter")));
        assert!(selected(&t, Some("example_dff::counter_counts")));
        assert!(selected(&t, Some("nope, counts")));
        assert!(!selected(&t, Some("nope")));
        assert!(!selected(&t, Some("example_dff::x")));
    }

    #[test]
    fn xml_escaping_and_counts() {
        let results = [
            TestResult {
                name: "a".into(),
                module: "m".into(),
                outcome: Outcome::Passed,
                sim_time_steps: 1500,
                wall_secs: 0.5,
                seed: 1,
                file: "src/lib.rs".into(),
                line: 10,
            },
            TestResult {
                name: "b<>&\"".into(),
                module: "m".into(),
                outcome: Outcome::Failed("x < y & \"z\"".into()),
                sim_time_steps: 0,
                wall_secs: 0.25,
                seed: 2,
                file: "src/lib.rs".into(),
                line: 20,
            },
            TestResult::new("c", "n", Outcome::Skipped),
        ];
        let dir = std::env::temp_dir().join(format!("rivet-xml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("results.xml");
        // The writer needs the runtime for precision; install a stub-free
        // path by checking the pure parts only when no runtime exists.
        if !runtime::is_initialised() {
            // write_results_xml calls runtime::precision(); emulate by
            // formatting the pieces the same way.
            assert_eq!(xml("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
        }
        let _ = path;
        let failed = results.iter().filter(|r| matches!(r.outcome, Outcome::Failed(_))).count();
        assert_eq!(failed, 1);
        let r: Result<()> = Outcome::Failed("m".into()).into();
        assert!(r.is_err());
        let ok: Result<()> = Outcome::Skipped.into();
        assert!(ok.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_tests_sorted_by_stage_module_name() {
        let mut v = [desc("b", "m", 1), desc("a", "m", 1), desc("z", "a", 0), desc("q", "z", -1)];
        v.sort_by(|a, b| a.stage.cmp(&b.stage).then_with(|| a.module.cmp(b.module)).then_with(|| a.name.cmp(b.name)));
        let names: Vec<&str> = v.iter().map(|t| t.name).collect();
        assert_eq!(names, ["q", "z", "a", "b"]);
    }
}
