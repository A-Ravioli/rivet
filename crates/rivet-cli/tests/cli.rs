//! End-to-end tests of the `rivet` command against the example crates.
//! Each test skips itself when the simulator it needs is not installed, so
//! `cargo test --workspace` stays green on a machine without EDA tools.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn rivet() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rivet"))
}

fn run(args: &[&str]) -> (i32, String) {
    let out = rivet().args(args).current_dir(repo_root()).output().expect("run rivet");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn results(example: &str, sim: &str) -> String {
    std::fs::read_to_string(repo_root().join("examples").join(example).join("sim_build").join(sim).join("results.xml"))
        .unwrap_or_default()
}

#[test]
fn icarus_conformance_run_and_results() {
    if !have("iverilog") {
        eprintln!("skipping: iverilog not installed");
        return;
    }
    let (code, text) = run(&["run", "--sim", "icarus", "-C", "examples/conformance"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("RIVET_RESULT passed=21 failed=0"), "{text}");
    let xml = results("conformance", "icarus");
    assert!(xml.contains(r#"tests="21" failures="0""#), "{xml}");
    assert!(xml.contains(r#"<property name="simulator" value="icarus" />"#));
    // A second run is a cache hit for the HDL build (stamp present).
    assert!(repo_root().join("examples/conformance/sim_build/icarus/build.hash").exists());
}

#[test]
fn icarus_filter_and_hdl_finish() {
    if !have("iverilog") {
        eprintln!("skipping: iverilog not installed");
        return;
    }
    // Only the selected test runs, and an HDL $finish while it runs is a failure with exit code 1.
    let (code, text) =
        run(&["run", "--sim", "icarus", "-C", "examples/conformance", "--filter", "hdl_finish", "--", "+finish_early"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("running 1 test(s)"), "{text}");
    assert!(text.contains("ended prematurely") || text.contains("simulator ended"), "{text}");
    assert!(text.contains("RIVET_RESULT passed=0 failed=1"), "{text}");
    let xml = results("conformance", "icarus");
    assert!(xml.contains(r#"tests="1" failures="1""#), "{xml}");
}

#[test]
fn icarus_waves_and_bindgen() {
    if !have("iverilog") {
        eprintln!("skipping: iverilog not installed");
        return;
    }
    let (code, text) = run(&["run", "--sim", "icarus", "-C", "examples/dff", "--waves", "--filter", "counter"]);
    assert_eq!(code, 0, "{text}");
    assert!(repo_root().join("examples/dff/sim_build/icarus/dff.fst").exists(), "FST waveform written");
    let out = std::env::temp_dir().join(format!("rivet-bindgen-{}.rs", std::process::id()));
    let (code, text) = run(&["bindgen", "--sim", "icarus", "-C", "examples/dff", "-o", out.to_str().unwrap()]);
    assert_eq!(code, 0, "{text}");
    let generated = std::fs::read_to_string(&out).unwrap();
    syn::parse_file(&generated).expect("generated bindings parse");
    assert!(generated.contains("pub struct Dff {"));
    assert!(generated.contains("pub mem: Vec<Signal>,"));
    let _ = std::fs::remove_file(&out);
}

#[test]
fn cli_errors() {
    let (code, text) = run(&["run", "--sim", "nonesuch", "-C", "examples/dff"]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("unsupported simulator"), "{text}");
    let (code, text) = run(&["run", "--sim", "icarus", "-C", "crates"]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("no package") || text.contains("rivet.toml"), "{text}");
    let (code, _) = run(&["frobnicate"]);
    assert_eq!(code, 2);
}

#[test]
fn verilator_conformance() {
    if !have("verilator") {
        eprintln!("skipping: verilator not installed");
        return;
    }
    let (code, text) = run(&["run", "--sim", "verilator", "-C", "examples/conformance"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("RIVET_RESULT passed=21 failed=0"), "{text}");
}

#[test]
fn ghdl_vhdl_example() {
    if !have("ghdl") {
        eprintln!("skipping: ghdl not installed");
        return;
    }
    let (code, text) = run(&["run", "--sim", "ghdl", "-C", "examples/dff_vhdl"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("RIVET_RESULT passed=2 failed=0"), "{text}");
}

/// The bus example on Icarus: parameter sets, sharding, merged results and
/// coverage, golden traces, and the coverage threshold.
#[test]
fn icarus_bus_example_sharded_with_param_sets_and_coverage() {
    if !have("iverilog") {
        eprintln!("skipping: iverilog not installed");
        return;
    }
    let (code, text) = run(&["run", "--sim", "icarus", "-C", "examples/bus", "-j", "3", "--cov-threshold", "99"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("parameter set init0"), "{text}");
    assert!(text.contains("parameter set init5"), "{text}");
    assert!(text.contains("in 3 shard(s)"), "{text}");
    assert!(text.contains("RIVET_RESULT passed=25 failed=0"), "{text}");
    assert!(text.contains("functional coverage 100.00%"), "{text}");
    let xml = results("bus", "icarus");
    assert!(xml.contains(r#"<testcase name="regs_reset_to_param@init5""#), "{xml}");
    assert!(!xml.contains(r#"<testcase name="regs_reset_to_param@init0""#), "param_sets restricts the test");
    assert!(xml.contains(r#"<testcase name="axi_mem_bursts[16]@init0""#), "params register one test per value");
    assert!(xml.contains(r#"<property name="random_seed""#));
    // An unreachable threshold fails the run without hiding results.
    let (code, text) = run(&[
        "run",
        "--sim",
        "icarus",
        "-C",
        "examples/bus",
        "--param-set",
        "init0",
        "--filter",
        "apb",
        "--cov-threshold",
        "50",
    ]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("no coverage recorded"), "{text}");
    // cov report merges and applies its own threshold.
    let (code, text) = run(&["cov", "report", "-C", "examples/bus", "--sim", "icarus", "--threshold", "100"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("TOTAL 36/36 bins (100.00%)"), "{text}");
    // A changed golden is reported with a diff and the actual trace.
    let golden = repo_root().join("examples/bus/golden/example_bus_axil_golden_trace@init0__axil.trace");
    let saved = std::fs::read_to_string(&golden).unwrap();
    std::fs::write(&golden, saved.replace("W 0x", "W 0y")).unwrap();
    let (code, text) =
        run(&["run", "--sim", "icarus", "-C", "examples/bus", "--param-set", "init0", "--filter", "golden"]);
    std::fs::write(&golden, &saved).unwrap();
    let _ = std::fs::remove_file(golden.with_extension("trace.actual"));
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("trace axil differs from"), "{text}");
    assert!(text.contains("-      1") && text.contains("+      1"), "{text}");
}

#[test]
fn watch_runs_once_and_cargo_rivet_alias() {
    if !have("iverilog") {
        eprintln!("skipping: iverilog not installed");
        return;
    }
    let out = rivet()
        .args(["watch", "--sim", "icarus", "-C", "examples/dff", "--filter", "counter"])
        .env("RIVET_WATCH_ONCE", "1")
        .current_dir(repo_root())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{text}");
    assert!(text.contains("watch: run finished (ok)"), "{text}");
    let out = Command::new(env!("CARGO_BIN_EXE_cargo-rivet"))
        .args(["rivet", "run", "--sim", "icarus", "-C", "examples/dff", "--filter", "counter", "--log-format", "json"])
        .current_dir(repo_root())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{text}");
    assert!(text.contains(r#""level":"INFO""#), "json logs: {text}");
    let log = repo_root().join("examples/dff/sim_build/icarus/logs/example_dff__counter_counts.log");
    assert!(log.exists(), "per-test log file");
}

#[test]
fn bindgen_emits_typedefs() {
    if !have("iverilog") {
        eprintln!("skipping: iverilog not installed");
        return;
    }
    let out = std::env::temp_dir().join(format!("rivet-bindgen-bus-{}.rs", std::process::id()));
    let (code, text) = run(&["bindgen", "--sim", "icarus", "-C", "examples/bus", "-o", out.to_str().unwrap()]);
    assert_eq!(code, 0, "{text}");
    let generated = std::fs::read_to_string(&out).unwrap();
    syn::parse_file(&generated).expect("generated bindings parse");
    assert!(generated.contains("pub enum Op {"), "{generated}");
    assert!(generated.contains("pub struct Cmd {"), "{generated}");
    assert!(generated.contains("pub fn hierarchy(&self) -> String"));
    assert!(!generated.contains("ivl_"), "no Icarus-internal scopes");
    // The committed bindings are current.
    let committed = std::fs::read_to_string(repo_root().join("examples/bus/src/dut.rs")).unwrap();
    assert_eq!(committed, generated, "examples/bus/src/dut.rs is stale; rerun rivet bindgen");
    let _ = std::fs::remove_file(&out);
}
