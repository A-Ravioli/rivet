//! Test registration and the regression loop.
//!
//! `#[rivet::test]` expands to an `inventory::submit!` of a [`TestDesc`].
//! At StartOfSim the runtime spawns [`run_regression`], which runs each
//! test in turn inside one simulator process, one time step apart, and
//! writes a JUnit `results.xml` compatible with cocotb's.

use crate::error::{Error, Result};
use crate::handle::Module;
use crate::runtime::{self, TestFailed};
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
    /// Tests are stable-sorted by stage.
    pub stage: i32,
}

inventory::collect!(TestDesc);

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
}

/// All tests visible to this binary, in execution order.
pub fn all_tests() -> Vec<&'static TestDesc> {
    let mut v: Vec<&TestDesc> = inventory::iter::<TestDesc>.into_iter().collect();
    // Link order is not deterministic across builds; sort by stage, then
    // module and name, so runs on different simulators agree.
    v.sort_by(|a, b| a.stage.cmp(&b.stage).then_with(|| a.module.cmp(b.module)).then_with(|| a.name.cmp(b.name)));
    v
}

/// Filter from `RIVET_TEST_FILTER`: comma-separated substrings; a test
/// runs if any matches its `module::name` or name.
fn selected(t: &TestDesc) -> bool {
    match std::env::var("RIVET_TEST_FILTER") {
        Ok(f) if !f.trim().is_empty() => f
            .split(',')
            .map(str::trim)
            .any(|pat| t.name == pat || t.name.contains(pat) || format!("{}::{}", t.module, t.name).contains(pat)),
        _ => true,
    }
}

/// Run every registered test. Spawned by the runtime at StartOfSim.
pub async fn run_regression(root: Module) -> Vec<TestResult> {
    let tests = all_tests();
    let precision = runtime::precision();
    let mut results = Vec::new();
    let total = tests.iter().filter(|t| selected(t)).count();
    log::info!(
        "running {} test(s) on {}",
        total,
        runtime::with(|rt| format!("{} {}", rt.backend.name(), rt.backend.version()))
    );
    let mut idx = 0;
    for t in tests {
        if !selected(t) {
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
            });
            continue;
        }
        if idx > 1 {
            // One time step between tests, as in cocotb.
            Timer::steps(1).await;
        }
        log::info!("running {}::{} ({idx}/{total})", t.module, t.name);
        let wall = Instant::now();
        let start = runtime::now();
        runtime::begin_test();
        let timeout = match std::panic::catch_unwind(t.timeout) {
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
        // Tear down: cancel everything the test started, drop buffered writes.
        runtime::cancel_all_other_tasks();
        runtime::discard_pending_writes();
        let outcome = match (outcome, recorded) {
            (Outcome::Passed, Some(f)) => Outcome::Failed(f),
            (o, _) => o,
        };
        let outcome = match (t.expect_fail, outcome) {
            (true, Outcome::Failed(msg)) => {
                log::info!("{}::{} failed as expected: {msg}", t.module, t.name);
                Outcome::Passed
            }
            (true, Outcome::Passed) => Outcome::Failed("test passed but was expected to fail".into()),
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
        results.push(TestResult {
            name: t.name.to_string(),
            module: t.module.to_string(),
            outcome,
            sim_time_steps,
            wall_secs: wall.elapsed().as_secs_f64(),
        });
    }
    results
}

async fn run_one(handle: crate::task::JoinHandle<Result<()>>, timeout: Option<Duration>) -> Outcome {
    let body = async move {
        match first(handle, TestFailed).await {
            Either::Left(Ok(Ok(()))) => Outcome::Passed,
            Either::Left(Ok(Err(e))) => Outcome::Failed(e.to_string()),
            Either::Left(Err(e)) => Outcome::Failed(e.to_string()),
            Either::Right(msg) => Outcome::Failed(msg),
        }
    };
    match timeout {
        Some(d) => match first(body, Timer::new(d)).await {
            Either::Left(o) => o,
            Either::Right(()) => Outcome::Failed(format!("timed out after {d} of simulated time")),
        },
        None => body.await,
    }
}

/// Print the summary table and return the number of failures.
pub fn summarize(results: &[TestResult]) -> usize {
    let precision = runtime::precision();
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
    eprintln!("RIVET_RESULT passed={passed} failed={failed} skipped={skipped}");
    failed
}

/// Write cocotb-compatible JUnit XML.
pub fn write_results_xml(path: &std::path::Path, results: &[TestResult], sim_name: &str) -> std::io::Result<()> {
    use std::fmt::Write as _;
    let precision = runtime::precision();
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
                r#"    <testcase name="{}" classname="{}" file="" lineno="0" time="{:.6}">"#,
                xml(&r.name),
                xml(&r.module),
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
    let path = std::env::var("RIVET_RESULTS_FILE").unwrap_or_else(|_| "results.xml".to_string());
    if let Err(e) = write_results_xml(std::path::Path::new(&path), results, &sim) {
        log::error!("cannot write {path}: {e}");
    }
    runtime::with(|rt| rt.exit_code = if failed > 0 { 1 } else { 0 });
    runtime::finish();
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
