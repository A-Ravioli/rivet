//! `rivet`: build the test crate, compile the design, run the simulator
//! with the harness loaded, and report results.
//!
//! ```text
//! rivet run --sim icarus [--release] [--filter a,b] [--waves] [-p crate] [-C dir]
//! rivet run --sim verilator ...
//! rivet build --sim icarus
//! rivet clean
//! ```

use rivet_manifest::Manifest;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

struct Opts {
    cmd: String,
    sim: String,
    release: bool,
    filter: Option<String>,
    waves: bool,
    package: Option<String>,
    dir: PathBuf,
    extra: Vec<String>,
    verbose: bool,
    seed: Option<String>,
    log: Option<String>,
}

fn usage() -> ! {
    eprintln!(
        "usage: rivet <run|build|clean> [options] [-- sim args]\n\
         \n\
         options:\n\
         \x20 --sim <icarus|verilator>   simulator (default: icarus)\n\
         \x20 -p, --package <name>       test crate (default: crate in the current directory)\n\
         \x20 -C <dir>                   change to directory first\n\
         \x20 --release                  build the harness in release mode\n\
         \x20 --filter <a,b>             run only tests whose name contains one of these\n\
         \x20 --waves                    dump waveforms\n\
         \x20 --seed <n>                 random seed passed as RIVET_SEED\n\
         \x20 --log <level>              RIVET_LOG level (error|warn|info|debug|trace)\n\
         \x20 -v                         verbose"
    );
    std::process::exit(2)
}

fn parse_args() -> Opts {
    let mut o = Opts {
        cmd: String::new(),
        sim: "icarus".into(),
        release: false,
        filter: None,
        waves: false,
        package: None,
        dir: std::env::current_dir().unwrap(),
        extra: Vec::new(),
        verbose: false,
        seed: None,
        log: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sim" => o.sim = it.next().unwrap_or_else(|| usage()),
            "-p" | "--package" => o.package = it.next(),
            "-C" => o.dir = PathBuf::from(it.next().unwrap_or_else(|| usage())),
            "--release" => o.release = true,
            "--filter" | "-k" => o.filter = it.next(),
            "--waves" => o.waves = true,
            "--seed" => o.seed = it.next(),
            "--log" => o.log = it.next(),
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

fn hash_inputs(paths: &[PathBuf], extra: &[String]) -> Result<u64, String> {
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
        std::fs::write(
            &dump,
            format!(
                "module rivet_dump;\n  initial begin\n    $dumpfile(\"{}\");\n    $dumpvars(0, {});\n  end\nendmodule\n",
                wave_file.display(),
                m.design.top
            ),
        )
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

fn results_summary(path: &Path) -> Option<(usize, usize, usize)> {
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
    if opts.waves {
        cmd.env("RIVET_WAVES", "1");
    }
}

fn run(opts: &Opts) -> Result<ExitCode, String> {
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
        other => return Err(format!("unsupported simulator {other:?} (icarus, verilator)")),
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

fn main() -> ExitCode {
    let opts = parse_args();
    let r = match opts.cmd.as_str() {
        "run" | "build" => run(&opts),
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
