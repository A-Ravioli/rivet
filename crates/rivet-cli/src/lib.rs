//! `rivet`: build the test crate, compile the design, run the simulator
//! with the harness loaded, and report results.
//!
//! ```text
//! rivet run --sim icarus [--release] [--filter a,b] [--waves] [-p crate] [-C dir]
//! rivet run --sim verilator ...
//! rivet build --sim icarus
//! rivet clean
//! ```

pub mod bindgen;

use rivet_manifest::Manifest;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

pub struct Opts {
    pub cmd: String,
    pub sim: String,
    pub release: bool,
    pub filter: Option<String>,
    pub waves: bool,
    /// One waveform file per test (Verilator).
    pub waves_per_test: bool,
    pub package: Option<String>,
    pub dir: PathBuf,
    pub extra: Vec<String>,
    pub verbose: bool,
    pub seed: Option<String>,
    pub log: Option<String>,
    /// bindgen: write the hierarchy here instead of running tests.
    pub dump: Option<PathBuf>,
    pub out: Option<PathBuf>,
}

impl Opts {
    /// Defaults for running `cmd` in `dir`.
    pub fn new(cmd: &str, dir: PathBuf) -> Opts {
        Opts {
            cmd: cmd.into(),
            sim: std::env::var("RIVET_SIM").unwrap_or_else(|_| "icarus".into()),
            release: false,
            filter: None,
            waves: false,
            waves_per_test: false,
            package: None,
            dir,
            extra: Vec::new(),
            verbose: false,
            seed: None,
            log: None,
            dump: None,
            out: None,
        }
    }
}

pub fn usage() -> ! {
    eprintln!(
        "usage: rivet <run|build|clean> [options] [-- sim args]\n\
         \n\
         options:\n\
         \x20 --sim <icarus|verilator|ghdl>  simulator (default: icarus)\n\
         \x20 -p, --package <name>       test crate (default: crate in the current directory)\n\
         \x20 -C <dir>                   change to directory first\n\
         \x20 --release                  build the harness in release mode\n\
         \x20 --filter <a,b>             run only tests whose name contains one of these\n\
         \x20 --waves                    dump waveforms\n\
         \x20 --waves-per-test           one waveform file per test (Verilator)\n\
         \x20 --seed <n>                 random seed passed as RIVET_SEED\n\
         \x20 --log <level>              RIVET_LOG level (error|warn|info|debug|trace)\n\
         \x20 -o <file>                  bindgen: output file (default src/dut.rs)\n\
         \x20 -v                         verbose\n\
         \n\
         commands:\n\
         \x20 run       build everything and run the tests\n\
         \x20 build     build without running\n\
         \x20 bindgen   run the design once to dump its hierarchy, then write typed bindings\n\
         \x20 clean     remove sim_build"
    );
    std::process::exit(2)
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Opts {
    let mut o = Opts::new("", std::env::current_dir().unwrap());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sim" => o.sim = it.next().unwrap_or_else(|| usage()),
            "-p" | "--package" => o.package = it.next(),
            "-C" => o.dir = PathBuf::from(it.next().unwrap_or_else(|| usage())),
            "--release" => o.release = true,
            "--filter" | "-k" => o.filter = it.next(),
            "--waves" => o.waves = true,
            "--waves-per-test" => {
                o.waves = true;
                o.waves_per_test = true;
            }
            "--seed" => o.seed = it.next(),
            "--log" => o.log = it.next(),
            "-o" | "--out" => o.out = it.next().map(PathBuf::from),
            "-v" | "--verbose" => o.verbose = true,
            "-h" | "--help" => usage(),
            "--" => {
                o.extra.extend(it.by_ref());
                break;
            }
            s if s.starts_with('-') => {
                eprintln!("unknown option {s}");
                usage()
            }
            s if o.cmd.is_empty() => o.cmd = s.to_string(),
            s => {
                eprintln!("unexpected argument {s}");
                usage()
            }
        }
    }
    if o.cmd.is_empty() {
        usage();
    }
    o
}

struct Package {
    name: String,
    lib_name: String,
    manifest_dir: PathBuf,
    target_dir: PathBuf,
    verilator_bin: Option<String>,
}

fn cargo_metadata(dir: &Path, package: Option<&str>) -> Result<Package, String> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let target_dir = PathBuf::from(v["target_directory"].as_str().unwrap_or("target"));
    let packages = v["packages"].as_array().cloned().unwrap_or_default();
    // Pick the requested package, else the one whose manifest is in `dir`.
    let dir_manifest = dir.join("Cargo.toml");
    let dir_manifest = dir_manifest.canonicalize().unwrap_or(dir_manifest);
    let pkg = packages
        .iter()
        .find(|p| match package {
            Some(n) => p["name"].as_str() == Some(n),
            None => {
                let mp = PathBuf::from(p["manifest_path"].as_str().unwrap_or(""));
                mp.canonicalize().map(|m| m == dir_manifest).unwrap_or(false)
            }
        })
        .ok_or_else(|| match package {
            Some(n) => format!("package {n} not found in workspace"),
            None => format!("no package at {}; use -p or -C", dir.display()),
        })?;
    let name = pkg["name"].as_str().unwrap().to_string();
    let manifest_dir = PathBuf::from(pkg["manifest_path"].as_str().unwrap()).parent().unwrap().to_path_buf();
    let mut lib_name = name.replace('-', "_");
    let mut verilator_bin = None;
    for t in pkg["targets"].as_array().cloned().unwrap_or_default() {
        let kinds: Vec<&str> =
            t["kind"].as_array().map(|k| k.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
        if kinds.contains(&"cdylib") {
            lib_name = t["name"].as_str().unwrap_or(&lib_name).replace('-', "_");
        }
        if kinds.contains(&"bin") {
            let rf: Vec<&str> = t["required-features"]
                .as_array()
                .map(|k| k.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            if rf.contains(&"verilator") || verilator_bin.is_none() {
                verilator_bin = Some(t["name"].as_str().unwrap().to_string());
            }
        }
    }
    Ok(Package { name, lib_name, manifest_dir, target_dir, verilator_bin })
}

fn run_cmd(mut cmd: Command, verbose: bool) -> Result<(), String> {
    if verbose {
        eprintln!("rivet: {cmd:?}");
    }
    let status = cmd.status().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if !status.success() {
        return Err(format!("{:?} failed with {status}", cmd.get_program()));
    }
    Ok(())
}

/// The companion module compiled in for `--waves` on Icarus: dumps to
/// `+rivet_wave=<file>` (default `default_file`) and lets the harness pause
/// and resume dumping through `rivet_dump_enable` (`rivet::waves`).
pub fn dump_module(default_file: &str, top: &str) -> String {
    format!(
        "module rivet_dump;\n  \
           reg rivet_dump_enable = 1;\n  \
           string rivet_dump_file;\n  \
           initial begin\n    \
             if (!$value$plusargs(\"rivet_wave=%s\", rivet_dump_file)) rivet_dump_file = \"{default_file}\";\n    \
             $dumpfile(rivet_dump_file);\n    \
             $dumpvars(0, {top});\n  \
           end\n  \
           always @(rivet_dump_enable) if (rivet_dump_enable) $dumpon; else $dumpoff;\n\
         endmodule\n"
    )
}

pub fn hash_inputs(paths: &[PathBuf], extra: &[String]) -> Result<u64, String> {
    let mut h = DefaultHasher::new();
    for p in paths {
        let data = std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
        p.hash(&mut h);
        data.hash(&mut h);
    }
    extra.hash(&mut h);
    Ok(h.finish())
}

fn build_icarus(m: &Manifest, opts: &Opts, sim_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(sim_dir).map_err(|e| e.to_string())?;
    let cfg = m.sim("icarus");
    let sources = m.sources_abs();
    let out = sim_dir.join(format!("{}.vvp", m.design.top));
    let mut args: Vec<String> = vec!["-o".into(), out.display().to_string(), "-s".into(), m.design.top.clone()];
    if !cfg.args.iter().any(|a| a.starts_with("-g")) {
        args.push("-g2012".into());
    }
    for i in m.includes_abs() {
        args.push(format!("-I{}", i.display()));
    }
    for (k, v) in &m.design.defines {
        args.push(format!("-D{k}={v}"));
    }
    for (k, v) in &m.design.params {
        args.push(format!("-P{}.{k}={v}", m.design.top));
    }
    args.extend(cfg.args.iter().cloned());
    let mut all_sources = sources.clone();
    if opts.waves {
        // Icarus needs $dumpvars in the design; generate a companion module
        // as cocotb does.
        let dump = sim_dir.join("rivet_dump.sv");
        let wave_file = sim_dir.join(format!("{}.fst", m.design.top));
        std::fs::write(&dump, dump_module(&wave_file.display().to_string(), &m.design.top))
            .map_err(|e| e.to_string())?;
        args.push("-s".into());
        args.push("rivet_dump".into());
        all_sources.push(dump);
    }
    if let Some(ts) = &m.design.timescale {
        let cmds = sim_dir.join("cmds.f");
        std::fs::write(&cmds, format!("+timescale+{ts}\n")).map_err(|e| e.to_string())?;
        args.push("-f".into());
        args.push(cmds.display().to_string());
    }
    let hash = hash_inputs(&all_sources, &args)?;
    let stamp = sim_dir.join("build.hash");
    if out.exists() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(&hash.to_string()) {
        if opts.verbose {
            eprintln!("rivet: {} up to date", out.display());
        }
        return Ok(out);
    }
    let mut cmd = Command::new("iverilog");
    cmd.args(&args).args(&all_sources);
    run_cmd(cmd, opts.verbose)?;
    std::fs::write(&stamp, hash.to_string()).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Analyse and elaborate with GHDL (mcode backend: nothing to link, the
/// design runs with `ghdl -r`).
fn build_ghdl(m: &Manifest, opts: &Opts, sim_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(sim_dir).map_err(|e| e.to_string())?;
    let cfg = m.sim("ghdl");
    let sources = m.sources_abs();
    let mut common: Vec<String> = vec![format!("--workdir={}", sim_dir.display())];
    if !cfg.args.iter().any(|a| a.starts_with("--std")) {
        common.push("--std=08".into());
    }
    common.extend(cfg.args.iter().cloned());
    let hash = hash_inputs(&sources, &common)?;
    let stamp = sim_dir.join("build.hash");
    if std::fs::read_to_string(&stamp).ok().as_deref() == Some(&hash.to_string()) {
        return Ok(());
    }
    let mut cmd = Command::new("ghdl");
    cmd.arg("-a").args(&common).args(&sources);
    run_cmd(cmd, opts.verbose)?;
    let mut cmd = Command::new("ghdl");
    cmd.arg("-e").args(&common).arg(&m.design.top);
    run_cmd(cmd, opts.verbose)?;
    std::fs::write(&stamp, hash.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

fn cargo_build(pkg: &Package, opts: &Opts, verilator: bool) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("-p").arg(&pkg.name).current_dir(&pkg.manifest_dir);
    if opts.release {
        cmd.arg("--release");
    }
    if verilator {
        let bin = pkg.verilator_bin.as_ref().ok_or("no [[bin]] target for Verilator in this crate")?;
        cmd.args(["--features", "verilator", "--bin", bin]);
    } else {
        cmd.arg("--lib");
    }
    run_cmd(cmd, opts.verbose)
}

/// `(tests, failures, skipped)` from a results.xml.
pub fn results_summary(path: &Path) -> Option<(usize, usize, usize)> {
    let xml = std::fs::read_to_string(path).ok()?;
    let mut tests = 0;
    let mut failures = 0;
    let mut skipped = 0;
    for line in xml.lines() {
        let line = line.trim_start();
        if line.starts_with("<testsuite ") {
            let attr = |k: &str| -> usize {
                line.split(&format!("{k}=\""))
                    .nth(1)
                    .and_then(|s| s.split('"').next())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0)
            };
            tests += attr("tests");
            failures += attr("failures") + attr("errors");
            skipped += attr("skipped");
        }
    }
    Some((tests, failures, skipped))
}

fn common_env(cmd: &mut Command, opts: &Opts, m: &Manifest, results: &Path) {
    cmd.env("RIVET_RESULTS_FILE", results);
    cmd.env("RIVET_TOPLEVEL", &m.design.top);
    if let Some(f) = &opts.filter {
        cmd.env("RIVET_TEST_FILTER", f);
    }
    if let Some(s) = &opts.seed {
        cmd.env("RIVET_SEED", s);
    }
    if let Some(l) = &opts.log {
        cmd.env("RIVET_LOG", l);
    }
    if opts.waves_per_test {
        cmd.env("RIVET_WAVES", "per-test");
    } else if opts.waves {
        cmd.env("RIVET_WAVES", "1");
    }
    if let Some(d) = &opts.dump {
        cmd.env("RIVET_DUMP_HIERARCHY", d);
    }
}

/// Run `build`, `run`, or a `bindgen` dump according to `opts`.
pub fn run(opts: &Opts) -> Result<ExitCode, String> {
    let pkg = cargo_metadata(&opts.dir, opts.package.as_deref())?;
    let m = Manifest::find(&pkg.manifest_dir)?;
    let sim_dir = pkg.manifest_dir.join("sim_build").join(&opts.sim);
    std::fs::create_dir_all(&sim_dir).map_err(|e| e.to_string())?;
    let results = sim_dir.join("results.xml");
    let _ = std::fs::remove_file(&results);
    let profile = if opts.release { "release" } else { "debug" };
    let build_only = opts.cmd == "build";

    match opts.sim.as_str() {
        "icarus" => {
            cargo_build(&pkg, opts, false)?;
            let vvp = build_icarus(&m, opts, &sim_dir)?;
            if build_only {
                return Ok(ExitCode::SUCCESS);
            }
            let so = pkg.target_dir.join(profile).join(format!("lib{}.so", pkg.lib_name));
            let plugin = sim_dir.join(format!("{}.vpi", pkg.lib_name));
            std::fs::copy(&so, &plugin).map_err(|e| format!("cannot copy {}: {e}", so.display()))?;
            let mut cmd = Command::new("vvp");
            cmd.arg("-M").arg(&sim_dir).arg("-m").arg(&pkg.lib_name).arg(&vvp);
            cmd.args(m.sim("icarus").run_args.iter());
            cmd.args(&opts.extra);
            common_env(&mut cmd, opts, &m, &results);
            cmd.stdin(Stdio::null());
            run_cmd(cmd, opts.verbose)?;
        }
        "ghdl" => {
            cargo_build(&pkg, opts, false)?;
            build_ghdl(&m, opts, &sim_dir)?;
            if build_only {
                return Ok(ExitCode::SUCCESS);
            }
            let so = pkg.target_dir.join(profile).join(format!("lib{}.so", pkg.lib_name));
            let cfg = m.sim("ghdl");
            let mut cmd = Command::new("ghdl");
            cmd.arg("-r").arg(format!("--workdir={}", sim_dir.display()));
            if !cfg.args.iter().any(|a| a.starts_with("--std")) {
                cmd.arg("--std=08");
            }
            cmd.args(cfg.args.iter().filter(|a| a.starts_with("--std") || a.starts_with("-P")));
            cmd.arg(&m.design.top).arg(format!("--vpi={}", so.display()));
            for (k, v) in &m.design.params {
                cmd.arg(format!("-g{k}={v}"));
            }
            if opts.waves {
                cmd.arg(format!("--wave={}", sim_dir.join(format!("{}.ghw", m.design.top)).display()));
            }
            cmd.args(cfg.run_args.iter());
            cmd.args(&opts.extra);
            common_env(&mut cmd, opts, &m, &results);
            cmd.stdin(Stdio::null());
            let status = cmd.status().map_err(|e| format!("cannot run ghdl: {e}"))?;
            if opts.verbose {
                eprintln!("rivet: simulator exited with {status}");
            }
        }
        "verilator" => {
            cargo_build(&pkg, opts, true)?;
            if build_only {
                return Ok(ExitCode::SUCCESS);
            }
            let bin = pkg.target_dir.join(profile).join(pkg.verilator_bin.as_ref().unwrap());
            let mut cmd = Command::new(&bin);
            if opts.waves {
                cmd.arg("--trace").arg("--trace-file").arg(sim_dir.join(format!("{}.vcd", m.design.top)));
            }
            cmd.args(m.sim("verilator").run_args.iter());
            cmd.args(&opts.extra);
            cmd.current_dir(&sim_dir);
            common_env(&mut cmd, opts, &m, &results);
            let status = cmd.status().map_err(|e| format!("cannot run {}: {e}", bin.display()))?;
            if opts.verbose {
                eprintln!("rivet: simulator exited with {status}");
            }
        }
        other => return Err(format!("unsupported simulator {other:?} (icarus, verilator, ghdl)")),
    }

    if let Some(dump) = &opts.dump {
        let json =
            std::fs::read_to_string(dump).map_err(|e| format!("no hierarchy dump at {}: {e}", dump.display()))?;
        let code = bindgen::generate(&json, None)?;
        let out = opts.out.clone().unwrap_or_else(|| pkg.manifest_dir.join("src").join("dut.rs"));
        std::fs::write(&out, code).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        eprintln!("rivet: wrote {}", out.display());
        return Ok(ExitCode::SUCCESS);
    }
    match results_summary(&results) {
        Some((tests, failures, skipped)) => {
            eprintln!("rivet: {tests} tests, {failures} failed, {skipped} skipped ({})", results.display());
            Ok(if failures > 0 { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        None => {
            Err(format!("simulator produced no {}; it probably died before the harness finished", results.display()))
        }
    }
}

/// The `rivet` command line.
pub fn main_with_args(args: impl IntoIterator<Item = String>) -> ExitCode {
    let mut opts = parse_args(args);
    let r = match opts.cmd.as_str() {
        "run" | "build" => run(&opts),
        "bindgen" => {
            opts.dump = Some(opts.dir.join("sim_build").join(&opts.sim).join("hierarchy.json"));
            run(&opts)
        }
        "clean" => {
            let dir = opts.dir.join("sim_build");
            let _ = std::fs::remove_dir_all(&dir);
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command {other}")),
    };
    match r {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rivet: error: {e}");
            ExitCode::from(2)
        }
    }
}

/// libtest-style selection: positional filters are substrings of
/// `module::name` (or exact matches with `--exact`); `--skip` patterns
/// exclude.
pub fn select_tests<'a>(
    tests: &'a [(String, String)],
    filters: &[String],
    skips: &[String],
    exact: bool,
) -> Vec<&'a (String, String)> {
    tests
        .iter()
        .filter(|(module, name)| {
            let full = format!("{module}::{name}");
            let m = if filters.is_empty() {
                true
            } else if exact {
                filters.iter().any(|f| *f == full || *f == *name)
            } else {
                filters.iter().any(|f| full.contains(f.as_str()))
            };
            m && !skips.iter().any(|s| full.contains(s.as_str()))
        })
        .collect()
}

/// Entry point for `cargo test` on a Rivet test crate: a libtest-compatible
/// `main` for a `[[test]]` target with `harness = false`.
///
/// Recognises `--list`, `--exact`, `--skip <pat>`, `--nocapture`,
/// `--test-threads N`, `--format`, `--ignored` and a positional filter.
/// `--list` answers from the in-process test registry; running uses the
/// simulator named by `RIVET_SIM` (default `icarus`).
pub fn harness_main(tests: Vec<(String, String)>) -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut list = false;
    let mut filters: Vec<String> = Vec::new();
    let mut skips: Vec<String> = Vec::new();
    let mut exact = false;
    let mut json = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--list" => list = true,
            "--exact" => exact = true,
            "--skip" => {
                if let Some(s) = args.next() {
                    skips.push(s);
                }
            }
            "--nocapture" | "--ignored" | "--include-ignored" | "--show-output" | "-q" | "--quiet" => {}
            "--test-threads" | "-Z" | "--logfile" | "--color" => {
                args.next();
            }
            "--format" => {
                json = args.next().as_deref() == Some("json");
            }
            s if s.starts_with("--format=") => json = s == "--format=json",
            s if s.starts_with("--test-threads=") || s.starts_with("--color=") || s.starts_with("-Z") => {}
            s if s.starts_with('-') => {
                eprintln!("rivet harness: ignoring unknown option {s}");
            }
            s => filters.push(s.to_string()),
        }
    }
    let selected = select_tests(&tests, &filters, &skips, exact);
    if list {
        for (module, name) in &selected {
            println!("{module}::{name}: test");
        }
        println!();
        println!("{} tests, 0 benchmarks", selected.len());
        return ExitCode::SUCCESS;
    }
    if selected.is_empty() {
        println!(
            "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; {} filtered out\n",
            tests.len()
        );
        return ExitCode::SUCCESS;
    }
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()));
    let mut opts = Opts::new("run", dir);
    opts.package = std::env::var("CARGO_PKG_NAME").ok();
    opts.filter = Some(selected.iter().map(|(m, n)| format!("{m}::{n}")).collect::<Vec<_>>().join(","));
    opts.release = std::env::var("PROFILE").map(|p| p == "release").unwrap_or(false) || cfg!(not(debug_assertions));
    println!("\nrunning {} tests on {}", selected.len(), opts.sim);
    let started = std::time::Instant::now();
    let result = run(&opts);
    let results_path = opts.dir.join("sim_build").join(&opts.sim).join("results.xml");
    let (passed, failed) = match results_summary(&results_path) {
        Some((t, f, s)) => (t - f - s, f),
        None => (0, selected.len()),
    };
    let status = match &result {
        Ok(_) if failed == 0 => "ok",
        _ => "FAILED",
    };
    if json {
        println!(
            "{{ \"type\": \"suite\", \"event\": \"{}\", \"passed\": {passed}, \"failed\": {failed} }}",
            if failed == 0 { "ok" } else { "failed" }
        );
    }
    println!(
        "\ntest result: {status}. {passed} passed; {failed} failed; 0 ignored; 0 measured; {} filtered out; finished in {:.2}s\n",
        tests.len() - selected.len(),
        started.elapsed().as_secs_f64()
    );
    match result {
        Ok(c) if failed == 0 => c,
        Ok(_) => ExitCode::from(101),
        Err(e) => {
            eprintln!("rivet: error: {e}");
            ExitCode::from(101)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(m: &str, n: &str) -> (String, String) {
        (m.into(), n.into())
    }

    #[test]
    fn selection() {
        let tests = vec![t("m", "alpha"), t("m", "beta"), t("n", "alpha_two")];
        let none: Vec<String> = vec![];
        assert_eq!(select_tests(&tests, &none, &none, false).len(), 3);
        let f = vec!["alpha".to_string()];
        assert_eq!(select_tests(&tests, &f, &none, false).len(), 2);
        assert_eq!(select_tests(&tests, &f, &none, true).len(), 1, "--exact matches the bare name");
        let f2 = vec!["m::alpha".to_string()];
        assert_eq!(select_tests(&tests, &f2, &none, true).len(), 1);
        let skip = vec!["two".to_string()];
        assert_eq!(select_tests(&tests, &f, &skip, false).len(), 1);
    }

    #[test]
    fn results_xml_summary() {
        let dir = std::env::temp_dir().join(format!("rivet-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("results.xml");
        std::fs::write(
            &p,
            r#"<testsuites>
  <testsuite name="a" tests="3" failures="1" errors="0" skipped="1">
  </testsuite>
  <testsuite name="b" tests="2" failures="0" errors="1" skipped="0">
  </testsuite>
</testsuites>"#,
        )
        .unwrap();
        assert_eq!(results_summary(&p), Some((5, 2, 1)));
        assert_eq!(results_summary(&dir.join("missing.xml")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn input_hash_changes_with_content_and_args() {
        let dir = std::env::temp_dir().join(format!("rivet-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.sv");
        std::fs::write(&f, "module a; endmodule").unwrap();
        let h1 = hash_inputs(std::slice::from_ref(&f), &["-g2012".into()]).unwrap();
        let h2 = hash_inputs(std::slice::from_ref(&f), &["-g2005".into()]).unwrap();
        std::fs::write(&f, "module a; wire x; endmodule").unwrap();
        let h3 = hash_inputs(std::slice::from_ref(&f), &["-g2012".into()]).unwrap();
        assert_ne!(h1, h2);
        assert_ne!(h1, h3);
        assert!(hash_inputs(&[dir.join("missing.sv")], &[]).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arg_parsing() {
        let o = parse_args(
            [
                "run",
                "--sim",
                "verilator",
                "-C",
                "/tmp/x",
                "--release",
                "--filter",
                "a,b",
                "--waves",
                "-p",
                "pkg",
                "--",
                "+foo",
            ]
            .map(String::from),
        );
        assert_eq!(o.cmd, "run");
        assert_eq!(o.sim, "verilator");
        assert_eq!(o.dir, PathBuf::from("/tmp/x"));
        assert!(o.release && o.waves);
        assert_eq!(o.filter.as_deref(), Some("a,b"));
        assert_eq!(o.package.as_deref(), Some("pkg"));
        assert_eq!(o.extra, vec!["+foo"]);
    }
}
