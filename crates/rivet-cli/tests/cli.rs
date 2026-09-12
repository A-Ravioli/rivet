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
