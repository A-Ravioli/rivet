//! `rivet`: build the test crate, compile the design, run the simulator
//! with the harness loaded, and report results.
//!
//! ```text
//! rivet run --sim icarus [--release] [--filter a,b] [--waves] [-j N] [-p crate] [-C dir]
//! rivet run --sim verilator --param-set w16 ...
//! rivet build --sim icarus
//! rivet watch --sim icarus
//! rivet cov report [--threshold 90] [coverage.json ...]
//! rivet bindgen --sim icarus
//! rivet clean
//! ```

pub mod bindgen;
pub mod cov;
pub mod new;
pub mod typedefs;
pub mod watch;

use rivet_core::test::{summarize_with, write_results_xml_with, Outcome, TestResult};
use rivet_manifest::Manifest;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

#[derive(Clone)]
pub struct Opts {
    pub cmd: String,
    /// Subcommand (`cov report`).
    pub sub: String,
    pub sim: String,
    pub release: bool,
    pub filter: Option<String>,
    pub waves: bool,
    /// One waveform file per test (Verilator).
    pub waves_per_test: bool,
    pub package: Option<String>,
    pub dir: PathBuf,
    pub extra: Vec<String>,
    /// Positional files (`cov report a.json b.json`).
    pub files: Vec<PathBuf>,
    pub verbose: bool,
    pub seed: Option<String>,
    pub log: Option<String>,
    pub log_format: Option<String>,
    /// Per-test log directory; default `sim_build/<sim>/logs`, `--no-log-dir` disables.
    pub log_dir: Option<PathBuf>,
    pub no_log_dir: bool,
    pub wall_timeout: Option<f64>,
    /// Simulator processes to run in parallel.
    pub jobs: usize,
    /// Only this parameter set (default: every set in the manifest).
    pub param_set: Option<String>,
    pub cov_threshold: Option<f64>,
    pub update_golden: bool,
    /// Shuffle the test order within each stage, reproducibly from the seed.
    pub shuffle: bool,
    /// Run the simulator's own GUI (commercial tools only).
    pub gui: bool,
    /// Open the waveform in a viewer when the run finishes.
    pub wave_open: bool,
    /// Put this run's results and logs in `sim_build/<sim>/<suffix>/` while
    /// the design build stays shared. Used when one process runs one test,
    /// as `cargo nextest` does.
    pub run_dir_suffix: Option<String>,
    /// bindgen: write the hierarchy here instead of running tests.
    pub dump: Option<PathBuf>,
    pub out: Option<PathBuf>,
    /// An explicit `rivet.toml` (default: found next to or above the crate).
    pub manifest: Option<PathBuf>,
    /// `rivet new --path <dir>`: depend on a Rivet checkout, not crates.io.
    pub rivet_path: Option<PathBuf>,
    /// The testbench is Python: load the prebuilt Python plugin instead of
    /// building a Rust crate. Implied by a `[python]` section in
    /// `rivet.toml`.
    pub python: bool,
    /// An explicit Python plugin library, overriding the search.
    pub plugin: Option<PathBuf>,
    /// Import these modules instead of the manifest's `[python] tests`.
    pub python_tests: Option<Vec<String>>,
}

impl Opts {
    /// Defaults for running `cmd` in `dir`.
    pub fn new(cmd: &str, dir: PathBuf) -> Opts {
        Opts {
            cmd: cmd.into(),
            sub: String::new(),
            sim: std::env::var("RIVET_SIM").unwrap_or_else(|_| "icarus".into()),
            release: false,
            filter: None,
            waves: false,
            waves_per_test: false,
            package: None,
            dir,
            extra: Vec::new(),
            files: Vec::new(),
            verbose: false,
            seed: None,
            log: None,
            log_format: None,
            log_dir: None,
            no_log_dir: false,
            wall_timeout: None,
            jobs: 1,
            param_set: None,
            cov_threshold: None,
            update_golden: false,
            shuffle: false,
            gui: false,
            wave_open: false,
            run_dir_suffix: None,
            dump: None,
            out: None,
            manifest: std::env::var("RIVET_MANIFEST").ok().filter(|s| !s.is_empty()).map(PathBuf::from),
            rivet_path: None,
            python: false,
            plugin: std::env::var("RIVET_PYTHON_PLUGIN").ok().filter(|s| !s.is_empty()).map(PathBuf::from),
            python_tests: None,
        }
    }
}

pub fn usage() -> ! {
    eprintln!(
        "usage: rivet <new|run|build|watch|bindgen|cov|clean> [options] [-- sim args]\n\
         \n\
         options:\n\
         \x20 --python                   the testbench is Python (implied by [python] in rivet.toml)\n\
         \x20 --plugin <path>            an explicit Python plugin library\n\
         \x20 --python-tests <a,b>       import these modules instead of [python] tests\n\
         \x20 --sim <name>               icarus (default), verilator, ghdl, nvc;\n\
         \x20                            questa, xcelium, vcs, riviera, dsim (unverified)\n\
         \x20 -p, --package <name>       test crate (default: crate in the current directory)\n\
         \x20 -C <dir>                   change to directory first\n\
         \x20 --release                  build the harness in release mode\n\
         \x20 --filter <a,b>             run only tests matching one of these regular expressions\n\
         \x20 -j, --jobs <n>             run tests in n simulator processes (tests must be independent)\n\
         \x20 --param-set <name>         run only this [design.param_sets] entry (default: all)\n\
         \x20 --waves                    dump waveforms\n\
         \x20 --waves-per-test           one waveform file per test (Verilator)\n\
         \x20 --seed <n>                 random seed (RIVET_SEED); printed by every run for replay\n\
         \x20 --wall-timeout <secs>      per-test wall-clock limit (RIVET_WALL_TIMEOUT)\n\
         \x20 --log <level>              RIVET_LOG level (error|warn|info|debug|trace)\n\
         \x20 --log-format <text|json>   log record format\n\
         \x20 --log-dir <dir>            per-test log files (default sim_build/<sim>/logs)\n\
         \x20 --no-log-dir               no per-test log files\n\
         \x20 --cov-threshold <pct>      fail the run below this functional coverage\n\
         \x20 --update-golden            rewrite golden trace files from this run\n\
         \x20 --shuffle                  shuffle test order within each stage (reproducible from --seed)\n\
         \x20 --gui                      run the simulator's own GUI (Questa, Xcelium, VCS)\n\
         \x20 --wave-open                open the waveform in a viewer when the run finishes\n\
         \x20 --manifest <rivet.toml>    design description (default: next to or above the crate)\n\
         \x20 --path <dir>               new: depend on a Rivet checkout instead of crates.io\n\
         \x20 -o <file>                  bindgen: output file (default src/dut.rs)\n\
         \x20 -v                         verbose\n\
         \n\
         commands:\n\
         \x20 new <name>   scaffold a testbench crate that runs as generated\n\
         \x20 run          build everything and run the tests\n\
         \x20 build        build without running\n\
         \x20 watch        run, then rerun whenever a source file changes\n\
         \x20 bindgen      run the design once to dump its hierarchy, then write typed bindings\n\
         \x20 cov report   merge coverage.json files (default: this crate's) and report\n\
         \x20              [--threshold <pct>] [files...]\n\
         \x20 clean        remove sim_build"
    );
    std::process::exit(2)
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Opts {
    let mut o = Opts::new("", std::env::current_dir().unwrap());
    let mut it = args.into_iter();
    let num = |v: Option<String>, what: &str| -> f64 {
        v.and_then(|s| s.parse().ok()).unwrap_or_else(|| {
            eprintln!("{what} needs a number");
            usage()
        })
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sim" => o.sim = it.next().unwrap_or_else(|| usage()),
            "-p" | "--package" => o.package = it.next(),
            "-C" => o.dir = PathBuf::from(it.next().unwrap_or_else(|| usage())),
            "--release" => o.release = true,
            "--filter" | "-k" => o.filter = it.next(),
            "-j" | "--jobs" => o.jobs = (num(it.next(), "--jobs") as usize).max(1),
            "--param-set" => o.param_set = it.next(),
            "--waves" => o.waves = true,
            "--waves-per-test" => {
                o.waves = true;
                o.waves_per_test = true;
            }
            "--seed" => o.seed = it.next(),
            "--wall-timeout" => o.wall_timeout = Some(num(it.next(), "--wall-timeout")),
            "--log" => o.log = it.next(),
            "--log-format" => o.log_format = it.next(),
            "--log-dir" => o.log_dir = it.next().map(PathBuf::from),
            "--no-log-dir" => o.no_log_dir = true,
            "--cov-threshold" | "--threshold" => o.cov_threshold = Some(num(it.next(), "--cov-threshold")),
            "--update-golden" => o.update_golden = true,
            "--shuffle" => o.shuffle = true,
            "--gui" => {
                o.gui = true;
                o.waves = true;
            }
            "--wave-open" => {
                o.wave_open = true;
                o.waves = true;
            }
            "-o" | "--out" => o.out = it.next().map(PathBuf::from),
            "--manifest" => o.manifest = it.next().map(PathBuf::from),
            "--path" => o.rivet_path = it.next().map(PathBuf::from),
            "--python" => o.python = true,
            "--python-tests" => {
                o.python_tests = it
                    .next()
                    .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect());
                o.python = true;
            }
            "--plugin" => {
                o.plugin = it.next().map(PathBuf::from);
                o.python = true;
            }
            "-v" | "--verbose" => o.verbose = true,
            "-h" | "--help" => usage(),
            "--" => {
                o.extra.extend(it.by_ref());
                break;
            }
            s if s.len() > 2 && s.starts_with("-j") && s[2..].chars().all(|c| c.is_ascii_digit()) => {
                o.jobs = s[2..].parse().unwrap_or(1);
            }
            s if s.starts_with('-') => {
                eprintln!("unknown option {s}");
                usage()
            }
            s if o.cmd.is_empty() => o.cmd = s.to_string(),
            s if (o.cmd == "cov" || o.cmd == "new") && o.sub.is_empty() => o.sub = s.to_string(),
            s if o.cmd == "cov" => o.files.push(PathBuf::from(s)),
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

impl Package {
    /// The stand-in for a cargo package when the testbench is Python.
    ///
    /// There is no crate to build: the harness is a prebuilt plugin that
    /// embeds an interpreter, and the "package" is the directory holding
    /// `rivet.toml`. Everything downstream — the build lock, `sim_build`,
    /// `results.xml`, the golden directory — works off that directory
    /// exactly as it does for a Rust crate.
    fn for_python(m: &Manifest) -> Package {
        Package {
            name: m.design.top.clone(),
            lib_name: "rivet_python".to_string(),
            manifest_dir: m.dir.clone(),
            target_dir: m.dir.join("sim_build"),
            verilator_bin: None,
        }
    }
}

/// Find the manifest and the package for this run.
///
/// Python mode is used when asked for, and otherwise whenever the
/// manifest has a `[python]` section and there is no crate to build —
/// which is the case a Python user is in.
fn resolve_package(opts: &Opts) -> Result<(Package, Manifest, bool), String> {
    let find_manifest = || -> Result<Manifest, String> {
        match &opts.manifest {
            Some(p) => Manifest::load(p),
            None => Manifest::find(&opts.dir),
        }
    };
    if opts.python {
        let m = find_manifest()?;
        return Ok((Package::for_python(&m), m, true));
    }
    match cargo_metadata(&opts.dir, opts.package.as_deref()) {
        Ok(pkg) => {
            let m = load_manifest(&pkg, opts)?;
            Ok((pkg, m, false))
        }
        Err(crate_err) => {
            // No crate here. If the manifest describes a Python
            // testbench, that is what the user meant.
            match find_manifest() {
                Ok(m) if m.python.is_some() => Ok((Package::for_python(&m), m, true)),
                _ => Err(crate_err),
            }
        }
    }
}

/// Where the Python plugin library is.
///
/// In order: `--plugin` or `RIVET_PYTHON_PLUGIN`; next to the `rivet`
/// binary; a Rivet checkout, building it if it is not built yet; the
/// installed `rivet` Python package.
fn python_plugin(opts: &Opts) -> Result<PathBuf, String> {
    let vhpi = matches!(opts.sim.as_str(), "nvc");
    if let Some(p) = &opts.plugin {
        if p.exists() {
            return Ok(p.clone());
        }
        return Err(format!("no Python plugin at {}", p.display()));
    }
    let file = cdylib_file("rivet_python");
    let mut tried = Vec::new();

    // Next to the rivet binary, which is how a release tarball ships it.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let c = dir.join(&file);
            if c.exists() && !vhpi {
                return Ok(c);
            }
            tried.push(c);
        }
    }

    // A checkout: build it if it is missing or out of date. `cargo` is
    // cheap to ask and does nothing when the library is current.
    if let Some(root) = checkout_root() {
        let ws = root.join("python").join("rivet");
        if ws.join("Cargo.toml").exists() {
            return build_python_plugin(&ws, opts, vhpi);
        }
        tried.push(ws.join("Cargo.toml"));
    }

    // An installed wheel carries the plugin in the package's `_bin`
    // directory, beside the `rivet` binary this may well be.
    if let Some(dir) = installed_rivet_package() {
        for c in [dir.join("_bin").join(&file), dir.join(&file)] {
            if c.exists() && !vhpi {
                return Ok(c);
            }
            tried.push(c);
        }
    }

    Err(format!(
        "cannot find the Python plugin ({file}). Looked in:\n{}\n\
         Build it from a checkout with `cargo build --release -p rivet-python-plugin` \
         (add `--no-default-features --features vhpi` for NVC), or point at it with \
         --plugin or RIVET_PYTHON_PLUGIN.",
        tried.iter().map(|p| format!("  {}", p.display())).collect::<Vec<_>>().join("\n")
    ))
}

/// The root of a Rivet checkout, found from this executable
/// (`<root>/target/<profile>/rivet`) or from the working directory.
fn checkout_root() -> Option<PathBuf> {
    let looks_right = |p: &Path| p.join("crates").join("rivet-core").is_dir() && p.join("python").is_dir();
    if let Ok(exe) = std::env::current_exe() {
        let mut d = exe.parent();
        while let Some(p) = d {
            if looks_right(p) {
                return Some(p.to_path_buf());
            }
            d = p.parent();
        }
    }
    let mut d = std::env::current_dir().ok();
    while let Some(p) = d {
        if looks_right(&p) {
            return Some(p);
        }
        d = p.parent().map(Path::to_path_buf);
    }
    None
}

/// Build the plugin from a checkout and return the library.
fn build_python_plugin(ws: &Path, opts: &Opts, vhpi: bool) -> Result<PathBuf, String> {
    // VPI and VHPI cannot share a library, so they do not share a target
    // directory either.
    let target = ws.join("target").join(if vhpi { "vhpi" } else { "vpi" });
    let profile = if opts.release { "release" } else { "debug" };
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("-p").arg("rivet-python-plugin");
    if opts.release {
        cmd.arg("--release");
    }
    if vhpi {
        cmd.args(["--no-default-features", "--features", "vhpi"]);
    }
    cmd.current_dir(ws).env("CARGO_TARGET_DIR", &target);
    if !opts.verbose {
        cmd.stdout(Stdio::null());
    }
    run_cmd(cmd, opts.verbose).map_err(|e| {
        format!("{e}\nbuilding the Python plugin needs a Python development install (libpython and its headers)")
    })?;
    let so = target.join(profile).join(cdylib_file("rivet_python"));
    if !so.exists() {
        return Err(format!("the plugin build produced no {}", so.display()));
    }
    Ok(so)
}

/// The directory of an installed `rivet` Python package, if there is one.
fn installed_rivet_package() -> Option<PathBuf> {
    let out = Command::new("python3")
        .args(["-c", "import rivet, os; print(os.path.dirname(rivet.__file__))"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    p.is_dir().then_some(p)
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

fn load_manifest(pkg: &Package, opts: &Opts) -> Result<Manifest, String> {
    match &opts.manifest {
        Some(p) => Manifest::load(p),
        None => Manifest::find(&pkg.manifest_dir),
    }
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

fn build_icarus(m: &Manifest, opts: &Opts, sim_dir: &Path, set: Option<&str>) -> Result<PathBuf, String> {
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
    for (k, v) in m.params_for(set) {
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

/// Analyse and elaborate with NVC. The design runs with `nvc -r`, which
/// loads the harness through `--load` (VHPI).
fn build_nvc(m: &Manifest, opts: &Opts, sim_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(sim_dir).map_err(|e| e.to_string())?;
    let cfg = m.sim("nvc");
    let sources = m.sources_abs();
    let mut common: Vec<String> = vec![format!("--work={}", sim_dir.join("work").display())];
    if !cfg.args.iter().any(|a| a.starts_with("--std")) {
        common.push("--std=2008".into());
    }
    common.extend(cfg.args.iter().cloned());
    let mut hash_inputs_extra = common.clone();
    // Generics are elaborated in, so a different parameter set is a
    // different build.
    for (k, v) in m.params_for(opts.param_set.as_deref()) {
        hash_inputs_extra.push(format!("-g{k}={v}"));
    }
    let hash = hash_inputs(&sources, &hash_inputs_extra)?;
    let stamp = sim_dir.join("build.hash");
    if std::fs::read_to_string(&stamp).ok().as_deref() == Some(&hash.to_string()) {
        return Ok(());
    }
    let mut cmd = Command::new("nvc");
    cmd.args(&common).arg("-a").args(&sources);
    run_cmd(cmd, opts.verbose)?;
    let mut cmd = Command::new("nvc");
    cmd.args(&common).arg("-e");
    for (k, v) in m.params_for(opts.param_set.as_deref()) {
        cmd.arg(format!("-g{k}={v}"));
    }
    // The elaborated design is saved into the work library, where `nvc -r`
    // finds it.
    cmd.arg(&m.design.top);
    run_cmd(cmd, opts.verbose)?;
    std::fs::write(&stamp, hash.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Build the test crate's `cdylib` with extra cargo features, for backends
/// the user's crate does not enable by default (VHPI).
/// Build flows for the simulators Rivet has never run on. Every flag comes
/// from cocotb's runner (`docs/design/00-cocotb-analysis.md` §5.1): the
/// design-access flags matter most, since without them the harness cannot
/// see any handles.
fn build_commercial(m: &Manifest, opts: &Opts, sim_dir: &Path, so: &Path) -> Result<Launch, String> {
    std::fs::create_dir_all(sim_dir).map_err(|e| e.to_string())?;
    let cfg = m.sim(&opts.sim);
    let sources: Vec<String> = m.sources_abs().iter().map(|p| p.display().to_string()).collect();
    let top = m.design.top.clone();
    let so = so.display().to_string();
    let params: Vec<(String, String)> = m.params_for(opts.param_set.as_deref()).into_iter().collect();
    let hash = hash_inputs(&m.sources_abs(), &cfg.args)?;
    let stamp = sim_dir.join("build.hash");
    let fresh = std::fs::read_to_string(&stamp).ok().as_deref() == Some(&hash.to_string());
    let run = |program: &str, args: Vec<String>| -> Result<(), String> {
        let mut cmd = Command::new(program);
        cmd.args(args).current_dir(sim_dir);
        run_cmd(cmd, opts.verbose)
    };
    let launch = match opts.sim.as_str() {
        "questa" => {
            if !fresh {
                let mut args = vec!["-work".into(), "work".into()];
                args.extend(cfg.args.iter().cloned());
                args.extend(sources.clone());
                run("vlog", args)?;
            }
            let mut args = vec![
                if opts.gui { "-gui".into() } else { "-c".into() },
                "-pli".into(),
                so.clone(),
                // Without full access the harness sees no handles.
                "-voptargs=-access=rw+/.".into(),
            ];
            for (k, v) in &params {
                args.push(format!("-g{k}={v}"));
            }
            args.extend(cfg.run_args.iter().cloned());
            args.push(format!("work.{top}"));
            args.push("-do".into());
            // In the GUI the user drives the run, so do not quit at the end.
            args.push(if opts.gui { "run -all".into() } else { "run -all; quit -f".to_string() });
            Launch::External { program: "vsim".into(), args }
        }
        "xcelium" => {
            // Single step: xrun compiles and runs.
            let mut args = vec![
                "-access".into(),
                "+rwc".into(),
                "-loadvpisim".into(),
                format!("{so}:vlog_startup_routines_bootstrap"),
                "-top".into(),
                top.clone(),
            ];
            for (k, v) in &params {
                args.push("-defparam".into());
                args.push(format!("{top}.{k}={v}"));
            }
            if opts.gui {
                args.push("-gui".into());
            }
            args.extend(cfg.args.iter().cloned());
            args.extend(sources.clone());
            args.extend(cfg.run_args.iter().cloned());
            Launch::External { program: "xrun".into(), args }
        }
        "vcs" => {
            if !fresh {
                // VCS loads the VPI library at compile time as well as at
                // run time (cocotb runner.py:276-279).
                let mut args = vec![
                    "-full64".into(),
                    "-sverilog".into(),
                    "+acc+3".into(),
                    "-debug_access+all".into(),
                    "-load".into(),
                    so.clone(),
                    "-o".into(),
                    "simv".into(),
                    "-top".into(),
                    top.clone(),
                ];
                for (k, v) in &params {
                    args.push(format!("-pvalue+{top}.{k}={v}"));
                }
                args.extend(cfg.args.iter().cloned());
                args.extend(sources.clone());
                run("vcs", args)?;
            }
            Launch::External { program: sim_dir.join("simv").display().to_string(), args: cfg.run_args.to_vec() }
        }
        "riviera" => {
            if !fresh {
                let mut args = vec!["-work".into(), "work".into()];
                args.extend(cfg.args.iter().cloned());
                args.extend(sources.clone());
                run("alog", args)?;
            }
            // Riviera drives the run from a .do script.
            let mut script = String::new();
            let generics: String = params.iter().map(|(k, v)| format!(" -g{k}={v}")).collect();
            script.push_str(&format!("asim -pli {so}{generics} work.{top}\nrun -all\nendsim\nquit -f\n"));
            let do_path = sim_dir.join("rivet.do");
            std::fs::write(&do_path, script).map_err(|e| e.to_string())?;
            Launch::External { program: "vsimsa".into(), args: vec!["-do".into(), do_path.display().to_string()] }
        }
        "dsim" => {
            let image = sim_dir.join("rivet.so").display().to_string();
            if !fresh {
                let mut args =
                    vec!["-genimage".into(), image.clone(), "-pli_lib".into(), so.clone(), "-top".into(), top.clone()];
                for (k, v) in &params {
                    args.push(format!("-defparam+{top}.{k}={v}"));
                }
                args.extend(cfg.args.iter().cloned());
                args.extend(sources.clone());
                run("dsim", args)?;
            }
            let mut args = vec!["-image".into(), image, "-pli_lib".into(), so];
            args.extend(cfg.run_args.iter().cloned());
            Launch::External { program: "dsim".into(), args }
        }
        other => return Err(format!("unsupported simulator {other:?}")),
    };
    std::fs::write(&stamp, hash.to_string()).map_err(|e| e.to_string())?;
    Ok(launch)
}

fn cargo_build_features(
    pkg: &Package,
    opts: &Opts,
    features: &[&str],
    no_default: bool,
    _set: Option<&str>,
) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("-p").arg(&pkg.name).arg("--lib").current_dir(&pkg.manifest_dir);
    if opts.release {
        cmd.arg("--release");
    }
    if no_default {
        // A VHPI simulator does not provide the VPI symbols, and it resolves
        // the library eagerly, so the cdylib must not carry both backends.
        cmd.arg("--no-default-features");
    }
    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }
    run_cmd(cmd, opts.verbose)
}

fn cargo_build(pkg: &Package, opts: &Opts, verilator: bool, set: Option<&str>) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("-p").arg(&pkg.name).current_dir(&pkg.manifest_dir);
    if opts.release {
        cmd.arg("--release");
    }
    if verilator {
        let bin = pkg.verilator_bin.as_ref().ok_or("no [[bin]] target for Verilator in this crate")?;
        cmd.args(["--features", "verilator", "--bin", bin]);
        // The Verilator model is parameterised at build time.
        cmd.env("RIVET_PARAM_SET", set.unwrap_or(""));
        if let Some(m) = &opts.manifest {
            cmd.env("RIVET_MANIFEST", std::fs::canonicalize(m).unwrap_or_else(|_| m.clone()));
        }
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

/// Parse a `results.json` written by the harness: `(results, precision, base seed)`.
pub fn read_results_json(path: &Path) -> Result<(Vec<TestResult>, i32, u64), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let precision = v["precision"].as_i64().unwrap_or(-12) as i32;
    let seed = v["seed"].as_u64().unwrap_or(0);
    let tests = v["tests"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|t| TestResult {
            name: t["name"].as_str().unwrap_or("?").to_string(),
            module: t["module"].as_str().unwrap_or("?").to_string(),
            outcome: match t["outcome"].as_str() {
                Some("passed") => Outcome::Passed,
                Some("skipped") => Outcome::Skipped,
                _ => Outcome::Failed(t["message"].as_str().unwrap_or("").to_string()),
            },
            sim_time_steps: t["sim_time_steps"].as_u64().unwrap_or(0),
            wall_secs: t["wall_secs"].as_f64().unwrap_or(0.0),
            seed: t["seed"].as_u64().unwrap_or(0),
            file: t["file"].as_str().unwrap_or("").to_string(),
            line: t["line"].as_u64().unwrap_or(0) as u32,
        })
        .collect();
    Ok((tests, precision, seed))
}

/// Split test names into `jobs` round-robin shards (no empty shards).
pub fn shard(names: &[String], jobs: usize) -> Vec<Vec<String>> {
    let n = jobs.max(1).min(names.len().max(1));
    let mut out: Vec<Vec<String>> = vec![Vec::new(); n];
    for (i, name) in names.iter().enumerate() {
        out[i % n].push(name.clone());
    }
    out.retain(|s| !s.is_empty());
    out
}

/// What a build produced and how to launch the simulator from it.
enum Launch {
    Icarus {
        vvp: PathBuf,
        plugin_dir: PathBuf,
        lib_name: String,
    },
    /// A simulator launched by a command line Rivet builds but has never
    /// run: the commercial tools. See `docs/SIMULATOR-QUIRKS.md`.
    External {
        program: String,
        args: Vec<String>,
    },
    Ghdl {
        so: PathBuf,
    },
    Nvc {
        so: PathBuf,
    },
    Verilator {
        bin: PathBuf,
    },
}

/// Environment every simulator run gets.
fn common_env(cmd: &mut Command, opts: &Opts, m: &Manifest, pkg: &Package, sim_dir: &Path, run_dir: &Path) {
    cmd.env("RIVET_RESULTS_FILE", run_dir.join("results.xml"));
    cmd.env("RIVET_RESULTS_JSON", run_dir.join("results.json"));
    cmd.env("RIVET_COVERAGE_FILE", run_dir.join("coverage.json"));
    cmd.env("RIVET_TOPLEVEL", &m.design.top);
    cmd.env("RIVET_GOLDEN_DIR", pkg.manifest_dir.join("golden"));
    if let Some(py) = &m.python {
        let tests = opts.python_tests.as_ref().unwrap_or(&py.tests);
        cmd.env("RIVET_PYTHON_TESTS", tests.join(","));
        // The manifest's directory first, then whatever it adds, so a
        // testbench sitting beside `rivet.toml` needs no configuration.
        let mut paths = vec![m.dir.clone()];
        paths.extend(py.paths.iter().map(|p| m.dir.join(p)));
        // Working from a checkout, the `rivet` package is in the tree
        // rather than installed. Put it last, so it is found only if
        // nothing earlier on the path provides it.
        if let Some(root) = checkout_root() {
            let src = root.join("python").join("rivet").join("src");
            if src.join("rivet").join("__init__.py").exists() {
                paths.push(src);
            }
        }
        let joined = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(":");
        cmd.env("RIVET_PYTHON_PATH", joined);
    }
    if let Some(s) = &opts.param_set {
        cmd.env("RIVET_PARAM_SET", s);
    }
    if let Some(f) = &opts.filter {
        cmd.env("RIVET_TEST_FILTER", f);
    }
    if let Some(s) = &opts.seed {
        cmd.env("RIVET_SEED", s);
    }
    if let Some(l) = &opts.log {
        cmd.env("RIVET_LOG", l);
    }
    if let Some(f) = &opts.log_format {
        cmd.env("RIVET_LOG_FORMAT", f);
    }
    if !opts.no_log_dir {
        cmd.env("RIVET_LOG_DIR", opts.log_dir.clone().unwrap_or_else(|| sim_dir.join("logs")));
    }
    if let Some(w) = opts.wall_timeout {
        cmd.env("RIVET_WALL_TIMEOUT", w.to_string());
    }
    if opts.update_golden {
        cmd.env("RIVET_UPDATE_GOLDEN", "1");
    }
    if opts.shuffle {
        cmd.env("RIVET_SHUFFLE", "1");
    }
    if opts.waves_per_test {
        cmd.env("RIVET_WAVES", "per-test");
    } else if opts.waves {
        cmd.env("RIVET_WAVES", "1");
    }
    if let Some(d) = &opts.dump {
        // Absolute: the simulator may run with its own working directory
        // (Verilator runs in the output directory).
        let abs = if d.is_absolute() {
            d.clone()
        } else {
            std::env::current_dir().map(|c| c.join(d)).unwrap_or_else(|_| d.clone())
        };
        if let Some(parent) = abs.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        cmd.env("RIVET_DUMP_HIERARCHY", abs);
    }
}

fn sim_command(launch: &Launch, opts: &Opts, m: &Manifest, sim_dir: &Path, run_dir: &Path) -> Command {
    match launch {
        Launch::Icarus { vvp, plugin_dir, lib_name } => {
            let mut cmd = Command::new("vvp");
            cmd.arg("-M").arg(plugin_dir).arg("-m").arg(lib_name).arg(vvp);
            cmd.args(m.sim("icarus").run_args.iter());
            if opts.waves && run_dir != sim_dir {
                cmd.arg(format!("+rivet_wave={}", run_dir.join(format!("{}.fst", m.design.top)).display()));
            }
            cmd.args(&opts.extra);
            cmd.stdin(Stdio::null());
            cmd
        }
        Launch::Ghdl { so } => {
            let cfg = m.sim("ghdl");
            let mut cmd = Command::new("ghdl");
            cmd.arg("-r").arg(format!("--workdir={}", sim_dir.display()));
            if !cfg.args.iter().any(|a| a.starts_with("--std")) {
                cmd.arg("--std=08");
            }
            cmd.args(cfg.args.iter().filter(|a| a.starts_with("--std") || a.starts_with("-P")));
            cmd.arg(&m.design.top).arg(format!("--vpi={}", so.display()));
            for (k, v) in m.params_for(opts.param_set.as_deref()) {
                cmd.arg(format!("-g{k}={v}"));
            }
            if opts.waves {
                cmd.arg(format!("--wave={}", run_dir.join(format!("{}.ghw", m.design.top)).display()));
            }
            cmd.args(cfg.run_args.iter());
            cmd.args(&opts.extra);
            cmd.stdin(Stdio::null());
            cmd
        }
        Launch::Nvc { so } => {
            let cfg = m.sim("nvc");
            let mut cmd = Command::new("nvc");
            cmd.arg(format!("--work={}", sim_dir.join("work").display()));
            if !cfg.args.iter().any(|a| a.starts_with("--std")) {
                cmd.arg("--std=2008");
            }
            cmd.arg("-r").arg("--load").arg(so);
            if opts.waves {
                cmd.arg("--wave").arg(run_dir.join(format!("{}.fst", m.design.top)));
            }
            cmd.arg(&m.design.top);
            cmd.args(cfg.run_args.iter());
            cmd.args(&opts.extra);
            cmd.stdin(Stdio::null());
            cmd
        }
        Launch::External { program, args } => {
            let mut cmd = Command::new(program);
            cmd.args(args);
            cmd.args(&opts.extra);
            cmd.current_dir(run_dir);
            cmd.stdin(Stdio::null());
            cmd
        }
        Launch::Verilator { bin } => {
            let mut cmd = Command::new(bin);
            if opts.waves {
                cmd.arg("--trace").arg("--trace-file").arg(run_dir.join(format!("{}.vcd", m.design.top)));
            }
            cmd.args(m.sim("verilator").run_args.iter());
            cmd.args(&opts.extra);
            cmd.current_dir(run_dir);
            cmd
        }
    }
}

/// The file Cargo writes for a `cdylib`: `lib<name>.so` on Linux and the
/// BSDs, `lib<name>.dylib` on macOS. Every simulator that loads the harness
/// as a PLI module is handed this file.
fn cdylib_file(lib_name: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{lib_name}.dylib")
    } else {
        format!("lib{lib_name}.so")
    }
}

/// Build the harness and the design for one parameter set.
fn build_one(pkg: &Package, m: &Manifest, opts: &Opts, sim_dir: &Path, set: Option<&str>) -> Result<Launch, String> {
    std::fs::create_dir_all(sim_dir).map_err(|e| e.to_string())?;
    let profile = if opts.release { "release" } else { "debug" };
    // A Python testbench has no crate to build: the harness is a prebuilt
    // plugin that embeds an interpreter.
    let harness = |verilator: bool| -> Result<PathBuf, String> {
        if opts.python {
            return python_plugin(opts);
        }
        cargo_build(pkg, opts, verilator, set)?;
        Ok(pkg.target_dir.join(profile).join(cdylib_file(&pkg.lib_name)))
    };
    match opts.sim.as_str() {
        "icarus" => {
            let so = harness(false)?;
            let vvp = build_icarus(m, opts, sim_dir, set)?;
            let plugin = sim_dir.join(format!("{}.vpi", pkg.lib_name));
            std::fs::copy(&so, &plugin).map_err(|e| format!("cannot copy {}: {e}", so.display()))?;
            Ok(Launch::Icarus { vvp, plugin_dir: sim_dir.to_path_buf(), lib_name: pkg.lib_name.clone() })
        }
        "ghdl" => {
            let so = harness(false)?;
            build_ghdl(m, opts, sim_dir)?;
            Ok(Launch::Ghdl { so })
        }
        "nvc" => {
            let so = if opts.python {
                python_plugin(opts)?
            } else {
                cargo_build_features(pkg, opts, &["vhpi"], true, set)?;
                pkg.target_dir.join(profile).join(cdylib_file(&pkg.lib_name))
            };
            build_nvc(m, opts, sim_dir)?;
            Ok(Launch::Nvc { so })
        }
        "verilator" => {
            if opts.python {
                // Verilator has no PLI to load a plugin through: the
                // harness is a binary that links the verilated model, so
                // it has to be compiled against the design.
                return Err("--python does not support Verilator yet; use icarus, ghdl or nvc".to_string());
            }
            cargo_build(pkg, opts, true, set)?;
            Ok(Launch::Verilator { bin: pkg.target_dir.join(profile).join(pkg.verilator_bin.as_ref().unwrap()) })
        }
        // Never executed: the flags come from cocotb's runner and the
        // behaviour from docs/SIMULATOR-QUIRKS.md. A licence holder running
        // examples/conformance is what turns these from code into support.
        "questa" | "xcelium" | "vcs" | "riviera" | "dsim" => {
            let so = harness(false)?;
            build_commercial(m, opts, sim_dir, &so)
        }
        other => Err(format!(
            "unsupported simulator {other:?} (icarus, verilator, ghdl, nvc; questa, xcelium, vcs, riviera and dsim are implemented but unverified)"
        )),
    }
}

/// Run one simulator process to completion, streaming its output.
fn run_sim(mut cmd: Command, opts: &Opts) -> Result<(), String> {
    if opts.verbose {
        eprintln!("rivet: {cmd:?}");
    }
    let status = cmd.status().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if opts.verbose {
        eprintln!("rivet: simulator exited with {status}");
    }
    Ok(())
}

/// Ask the harness which tests it would run (for sharding).
fn list_tests(
    launch: &Launch,
    opts: &Opts,
    m: &Manifest,
    pkg: &Package,
    sim_dir: &Path,
) -> Result<Vec<String>, String> {
    let list = sim_dir.join("tests.list");
    let _ = std::fs::remove_file(&list);
    let mut cmd = sim_command(launch, opts, m, sim_dir, sim_dir);
    common_env(&mut cmd, opts, m, pkg, sim_dir, sim_dir);
    cmd.env("RIVET_LIST_TESTS", &list);
    cmd.env_remove("RIVET_LOG_DIR");
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let status = cmd.status().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    let text = std::fs::read_to_string(&list)
        .map_err(|_| format!("the harness did not list its tests (simulator exited with {status})"))?;
    Ok(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
}

/// One run (one parameter set): serial or sharded across `opts.jobs`.
/// Returns the results, the time precision, and the coverage files.
/// Run the tests of one parameter set. `sim_dir` holds the build (the
/// simulator's own libraries and object files); `out_dir` receives results,
/// coverage, logs and waveforms, and is the same directory unless several
/// processes share one build.
fn run_set(
    launch: &Launch,
    opts: &Opts,
    m: &Manifest,
    pkg: &Package,
    sim_dir: &Path,
    out_dir: &Path,
) -> Result<(Vec<TestResult>, i32, Vec<PathBuf>), String> {
    if opts.jobs <= 1 {
        let mut cmd = sim_command(launch, opts, m, sim_dir, out_dir);
        common_env(&mut cmd, opts, m, pkg, sim_dir, out_dir);
        let _ = std::fs::remove_file(out_dir.join("results.json"));
        let _ = std::fs::remove_file(out_dir.join("coverage.json"));
        run_sim(cmd, opts)?;
        let json = out_dir.join("results.json");
        if !json.exists() {
            return Err(format!(
                "simulator produced no {}; it probably died before the harness finished",
                out_dir.join("results.xml").display()
            ));
        }
        let (results, precision, _) = read_results_json(&json)?;
        let cov = out_dir.join("coverage.json");
        return Ok((results, precision, if cov.exists() { vec![cov] } else { vec![] }));
    }
    let names = list_tests(launch, opts, m, pkg, sim_dir)?;
    let shards = shard(&names, opts.jobs);
    eprintln!("rivet: {} test(s) in {} shard(s)", names.len(), shards.len());
    let mut children = Vec::new();
    for (i, group) in shards.iter().enumerate() {
        let run_dir = out_dir.join(format!("shard{i}"));
        std::fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(run_dir.join("results.json"));
        let _ = std::fs::remove_file(run_dir.join("coverage.json"));
        let mut cmd = sim_command(launch, opts, m, sim_dir, &run_dir);
        common_env(&mut cmd, opts, m, pkg, sim_dir, &run_dir);
        cmd.env("RIVET_TEST_SELECT", group.join(","));
        let log = std::fs::File::create(run_dir.join("sim.log")).map_err(|e| e.to_string())?;
        cmd.stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?)).stderr(Stdio::from(log));
        if opts.verbose {
            eprintln!("rivet: shard {i}: {cmd:?}");
        }
        let child = cmd.spawn().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
        children.push((i, run_dir, child));
    }
    let mut all = Vec::new();
    let mut precision = -12;
    let mut cov_files = Vec::new();
    let mut errors = Vec::new();
    for (i, run_dir, mut child) in children {
        let status = child.wait().map_err(|e| e.to_string())?;
        let log = std::fs::read_to_string(run_dir.join("sim.log")).unwrap_or_default();
        eprintln!("rivet: ---- shard {i} ({status}) ----");
        eprint!("{log}");
        match read_results_json(&run_dir.join("results.json")) {
            Ok((r, p, _)) => {
                all.extend(r);
                precision = p;
            }
            Err(e) => errors.push(format!("shard {i}: {e}")),
        }
        let cov = run_dir.join("coverage.json");
        if cov.exists() {
            cov_files.push(cov);
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    // Present the merged list in the harness's order for equal stages.
    all.sort_by(|a, b| a.module.cmp(&b.module).then_with(|| a.name.cmp(&b.name)));
    Ok((all, precision, cov_files))
}

/// Run `build`, `run`, or a `bindgen` dump according to `opts`.
/// Open the waveform a run produced in whatever viewer is installed. The
/// open tools have no GUI of their own, so this is how `--wave-open` shows
/// a failing run.
fn open_waveform(dir: &Path, top: &str) {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for ext in ["fst", "vcd", "ghw"] {
        candidates.push(dir.join(format!("{top}.{ext}")));
    }
    // A per-test dump, if that is all there is.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if matches!(p.extension().and_then(|s| s.to_str()), Some("fst" | "vcd" | "ghw")) {
                candidates.push(p);
            }
        }
    }
    let Some(wave) = candidates.into_iter().find(|p| p.exists()) else {
        eprintln!("rivet: no waveform to open in {}", dir.display());
        return;
    };
    for viewer in ["surfer", "gtkwave"] {
        let ok = Command::new(viewer).arg(&wave).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        if let Ok(mut child) = ok {
            eprintln!("rivet: opened {} in {viewer}", wave.display());
            let _ = child.wait();
            return;
        }
    }
    eprintln!("rivet: no waveform viewer found (tried surfer and gtkwave); the dump is {}", wave.display());
}

/// A lock held while the design is built, so several harness processes
/// (one per test, as `cargo nextest` runs them) do not build into the same
/// directory at once.
struct BuildLock(PathBuf);

impl BuildLock {
    fn acquire(dir: &Path) -> BuildLock {
        let path = dir.join(".rivet-build.lock");
        let start = std::time::Instant::now();
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = writeln!(f, "{}", std::process::id());
                    return BuildLock(path);
                }
                Err(_) => {
                    // Take over a lock left behind by a process that died.
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map(|t| t.elapsed().map(|e| e.as_secs() > 900).unwrap_or(false))
                        .unwrap_or(false);
                    if stale || start.elapsed().as_secs() > 900 {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn run(opts: &Opts) -> Result<ExitCode, String> {
    let (pkg, m, python) = resolve_package(opts)?;
    let mut opts = opts.clone();
    opts.python = python;
    let opts = &opts;
    let no_tests = opts.python_tests.as_ref().map(|t| t.is_empty()).unwrap_or(true)
        && m.python.as_ref().map(|p| p.tests.is_empty()).unwrap_or(true);
    if python && no_tests {
        return Err(format!(
            "no Python test modules to import: add `tests = [\"test_something\"]` under [python] in {}",
            m.dir.join("rivet.toml").display()
        ));
    }
    let base_dir = pkg.manifest_dir.join("sim_build").join(&opts.sim);
    std::fs::create_dir_all(&base_dir).map_err(|e| e.to_string())?;
    // Where this run's results land. The build stays in `base_dir` so
    // processes running one test each share it.
    let out_base = match &opts.run_dir_suffix {
        Some(suffix) => {
            let d = base_dir.join(suffix);
            std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            d
        }
        None => base_dir.clone(),
    };
    let build_only = opts.cmd == "build";
    let sets: Vec<Option<String>> = match &opts.param_set {
        Some(s) => {
            if !m.design.param_sets.contains_key(s) {
                return Err(format!("no [design.param_sets.{s}] in rivet.toml"));
            }
            vec![Some(s.clone())]
        }
        None if m.design.param_sets.is_empty() || opts.dump.is_some() => vec![None],
        None => m.design.param_sets.keys().map(|k| Some(k.clone())).collect(),
    };
    let multi = sets.len() > 1 || sets[0].is_some();
    // One base seed for every process of this run, so shards and parameter
    // sets are replayable together with `--seed`.
    let mut opts = opts.clone();
    let seed = match &opts.seed {
        Some(s) => rivet_core::random::parse_seed(s).ok_or_else(|| format!("--seed {s:?} is not an integer"))?,
        None => {
            let s = rivet_core::random::base_seed();
            opts.seed = Some(s.to_string());
            s
        }
    };
    rivet_core::random::set_base_seed(seed);
    let opts = &opts;
    if opts.dump.is_none() {
        eprintln!("rivet: seed {seed} (replay with --seed {seed})");
    }
    let mut all: Vec<TestResult> = Vec::new();
    let mut precision = -12;
    let mut cov_files = Vec::new();
    let _ = std::fs::remove_file(out_base.join("results.xml"));
    for set in &sets {
        let sim_dir = match set {
            Some(s) => base_dir.join(s),
            None => base_dir.clone(),
        };
        let mut set_opts = opts.clone();
        set_opts.param_set = set.clone();
        if let Some(s) = set {
            eprintln!("rivet: parameter set {s}: {:?}", m.params_for(Some(s)));
        }
        let launch = {
            let _lock = BuildLock::acquire(&base_dir);
            build_one(&pkg, &m, &set_opts, &sim_dir, set.as_deref())?
        };
        if build_only {
            continue;
        }
        if opts.dump.is_some() {
            let mut cmd = sim_command(&launch, &set_opts, &m, &sim_dir, &sim_dir);
            common_env(&mut cmd, &set_opts, &m, &pkg, &sim_dir, &sim_dir);
            run_sim(cmd, opts)?;
            continue;
        }
        // Results and logs go in their own directory when several processes
        // share one build.
        let out_dir = match set {
            Some(s) if opts.run_dir_suffix.is_some() => {
                let d = out_base.join(s);
                std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
                d
            }
            _ if opts.run_dir_suffix.is_some() => out_base.clone(),
            _ => sim_dir.clone(),
        };
        let (mut results, p, cov) = run_set(&launch, &set_opts, &m, &pkg, &sim_dir, &out_dir)?;
        if let Some(s) = set {
            for r in &mut results {
                r.name = format!("{}@{s}", r.name);
            }
        }
        all.extend(results);
        precision = p;
        cov_files.extend(cov);
    }
    if build_only {
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(dump) = &opts.dump {
        let json =
            std::fs::read_to_string(dump).map_err(|e| format!("no hierarchy dump at {}: {e}", dump.display()))?;
        let code = bindgen::generate(&json, None, &m.sources_abs())?;
        let out = opts.out.clone().unwrap_or_else(|| pkg.manifest_dir.join("src").join("dut.rs"));
        std::fs::write(&out, code).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        // Format the bindings when rustfmt is available, so a committed
        // `dut.rs` matches what `cargo fmt` would produce and regenerating
        // it does not show up as a diff.
        let fmt = Command::new("rustfmt")
            .arg("--edition")
            .arg("2021")
            .arg(&out)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Ok(st) = fmt {
            if !st.success() && opts.verbose {
                eprintln!("rivet: rustfmt declined to format {}", out.display());
            }
        }
        eprintln!("rivet: wrote {}", out.display());
        return Ok(ExitCode::SUCCESS);
    }
    let results_path = out_base.join("results.xml");
    if multi || opts.jobs > 1 {
        // Merged results for the whole run.
        write_results_xml_with(&results_path, &all, &opts.sim, precision).map_err(|e| e.to_string())?;
        eprintln!("rivet: merged results");
        summarize_with(&all, precision);
    }
    if opts.wave_open {
        open_waveform(&out_base, &m.design.top);
    }
    let mut code = ExitCode::SUCCESS;
    let failures = all.iter().filter(|r| matches!(r.outcome, Outcome::Failed(_))).count();
    let skipped = all.iter().filter(|r| r.outcome == Outcome::Skipped).count();
    eprintln!("rivet: {} tests, {failures} failed, {skipped} skipped ({})", all.len(), results_path.display());
    if failures > 0 {
        code = ExitCode::from(1);
    }
    let _ = std::fs::remove_file(out_base.join("coverage.json"));
    if !cov_files.is_empty() {
        let mut report = cov::Report::default();
        for f in &cov_files {
            report.merge_file(f)?;
        }
        let merged = out_base.join("coverage.json");
        if cov_files.len() > 1 || cov_files[0] != merged {
            std::fs::write(&merged, report.to_json()).map_err(|e| e.to_string())?;
            eprintln!("rivet: merged coverage");
            eprint!("{}", report.render());
        }
        eprintln!("rivet: functional coverage {:.2}% ({})", report.percent(), merged.display());
        if let Some(t) = opts.cov_threshold {
            if report.percent() < t {
                eprintln!("rivet: coverage {:.2}% is below the threshold of {t}%", report.percent());
                code = ExitCode::from(1);
            }
        }
    } else if let Some(t) = opts.cov_threshold {
        eprintln!("rivet: no coverage recorded, threshold of {t}% not met");
        code = ExitCode::from(1);
    }
    Ok(code)
}

/// `rivet cov report`.
pub fn cov_report(opts: &Opts) -> Result<ExitCode, String> {
    let files: Vec<PathBuf> = if opts.files.is_empty() {
        let pkg = cargo_metadata(&opts.dir, opts.package.as_deref())?;
        let f = pkg.manifest_dir.join("sim_build").join(&opts.sim).join("coverage.json");
        if !f.exists() {
            return Err(format!("no {} (run the tests first, or pass coverage.json files)", f.display()));
        }
        vec![f]
    } else {
        opts.files.clone()
    };
    let mut report = cov::Report::default();
    for f in &files {
        report.merge_file(f)?;
    }
    print!("{}", report.render());
    if let Some(out) = &opts.out {
        std::fs::write(out, report.to_json()).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        eprintln!("rivet: wrote merged coverage to {}", out.display());
    }
    if let Some(t) = opts.cov_threshold {
        if report.percent() < t {
            eprintln!("rivet: coverage {:.2}% is below the threshold of {t}%", report.percent());
            return Ok(ExitCode::from(1));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `rivet watch`: run, then rerun on every change to the crate's sources
/// or the design.
pub fn watch(opts: &Opts) -> Result<ExitCode, String> {
    let pkg = cargo_metadata(&opts.dir, opts.package.as_deref())?;
    let m = load_manifest(&pkg, opts)?;
    let mut roots: Vec<PathBuf> = vec![
        pkg.manifest_dir.join("src"),
        pkg.manifest_dir.join("tests"),
        pkg.manifest_dir.join("golden"),
        pkg.manifest_dir.join("Cargo.toml"),
        pkg.manifest_dir.join("build.rs"),
        m.dir.join("rivet.toml"),
    ];
    roots.extend(m.sources_abs());
    roots.extend(m.includes_abs());
    roots.retain(|p| p.exists());
    let mut run_opts = opts.clone();
    run_opts.cmd = "run".into();
    let once = std::env::var("RIVET_WATCH_ONCE").is_ok();
    loop {
        let started = std::time::Instant::now();
        match run(&run_opts) {
            Ok(code) => eprintln!(
                "rivet: watch: run finished ({}) in {:.1}s; waiting for changes (Ctrl-C to stop)",
                if code == ExitCode::SUCCESS { "ok" } else { "failures" },
                started.elapsed().as_secs_f64()
            ),
            Err(e) => eprintln!("rivet: watch: error: {e}\nrivet: watch: waiting for changes (Ctrl-C to stop)"),
        }
        if once {
            return Ok(ExitCode::SUCCESS);
        }
        watch::wait_for_change(&roots);
        eprintln!("rivet: watch: change detected, rerunning");
    }
}

/// The `rivet` command line.
pub fn main_with_args(args: impl IntoIterator<Item = String>) -> ExitCode {
    let mut opts = parse_args(args);
    let r = match opts.cmd.as_str() {
        "new" => {
            if opts.sub.is_empty() {
                Err("rivet new needs a name: `rivet new my-tb`".to_string())
            } else {
                new::scaffold(&opts.sub, &opts.dir, opts.rivet_path.as_deref()).map(|root| {
                    println!(
                        "rivet: created {}\nrivet: next: cd {} && rivet run --sim icarus",
                        root.display(),
                        root.file_name().and_then(|s| s.to_str()).unwrap_or(".")
                    );
                    ExitCode::SUCCESS
                })
            }
        }
        "run" | "build" => run(&opts),
        "watch" => watch(&opts),
        "bindgen" => {
            let base = std::fs::canonicalize(&opts.dir).unwrap_or_else(|_| opts.dir.clone());
            opts.dump = Some(base.join("sim_build").join(&opts.sim).join("hierarchy.json"));
            run(&opts)
        }
        "cov" => match opts.sub.as_str() {
            "report" | "" => cov_report(&opts),
            other => Err(format!("unknown cov subcommand {other}; try `rivet cov report`")),
        },
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
/// `--test-threads N` (simulator processes), `--format`, `--ignored` and a
/// positional filter. `--list` answers from the in-process test registry;
/// running uses the simulator named by `RIVET_SIM` (default `icarus`).
pub fn harness_main(tests: Vec<(String, String)>) -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut list = false;
    let mut filters: Vec<String> = Vec::new();
    let mut skips: Vec<String> = Vec::new();
    let mut exact = false;
    let mut json = false;
    // `--list --format terse` must print only `name: test` lines: that is
    // what libtest does, and what `cargo nextest` parses.
    let mut terse = false;
    // libtest lists ignored tests separately, and `cargo nextest` asks for
    // that list to work out which tests to skip. Rivet has no ignored
    // tests: `skip` is decided by the harness and reported as a skip.
    let mut only_ignored = false;
    let mut jobs = 1usize;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--list" => list = true,
            "--exact" => exact = true,
            "--skip" => {
                if let Some(s) = args.next() {
                    skips.push(s);
                }
            }
            "--ignored" => only_ignored = true,
            "--nocapture" | "--include-ignored" | "--show-output" | "-q" | "--quiet" => {}
            "--test-threads" => {
                jobs = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            "-Z" | "--logfile" | "--color" => {
                args.next();
            }
            "--format" => {
                let f = args.next();
                json = f.as_deref() == Some("json");
                terse = f.as_deref() == Some("terse");
            }
            s if s.starts_with("--format=") => {
                json = s == "--format=json";
                terse = s == "--format=terse";
            }
            s if s.starts_with("--test-threads=") => {
                jobs = s["--test-threads=".len()..].parse().unwrap_or(1);
            }
            s if s.starts_with("--color=") || s.starts_with("-Z") => {}
            s if s.starts_with('-') => {
                eprintln!("rivet harness: ignoring unknown option {s}");
            }
            s => filters.push(s.to_string()),
        }
    }
    let selected = if only_ignored { Vec::new() } else { select_tests(&tests, &filters, &skips, exact) };
    if list {
        for (module, name) in &selected {
            println!("{module}::{name}: test");
        }
        if !terse {
            println!();
            println!("{} tests, 0 benchmarks", selected.len());
        }
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
    // `cargo nextest` runs one test per process, several at a time. Give
    // each its own results directory; the build is shared and locked.
    if selected.len() == 1 && tests.len() > 1 {
        let (module, name) = selected[0];
        let safe: String =
            format!("{module}__{name}").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        opts.run_dir_suffix = Some(format!("one/{safe}"));
    }
    opts.release = std::env::var("PROFILE").map(|p| p == "release").unwrap_or(false) || cfg!(not(debug_assertions));
    opts.jobs = std::env::var("RIVET_JOBS").ok().and_then(|j| j.parse().ok()).unwrap_or(jobs);
    println!("\nrunning {} tests on {}", selected.len(), opts.sim);
    let started = std::time::Instant::now();
    let result = run(&opts);
    let mut results_path = opts.dir.join("sim_build").join(&opts.sim);
    if let Some(suffix) = &opts.run_dir_suffix {
        results_path = results_path.join(suffix);
    }
    let results_path = results_path.join("results.xml");
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
    fn cdylib_extension_matches_the_platform() {
        let f = cdylib_file("demo");
        assert!(f.starts_with("libdemo."));
        // Cargo names a cdylib after the platform, and every simulator is
        // handed that exact file; getting this wrong fails the run with
        // "cannot copy ...: No such file or directory".
        if cfg!(target_os = "macos") {
            assert_eq!(f, "libdemo.dylib");
        } else {
            assert_eq!(f, "libdemo.so");
        }
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
    fn results_json_round_trip() {
        let dir = std::env::temp_dir().join(format!("rivet-cli-json-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("results.json");
        let results = [
            TestResult {
                name: "a".into(),
                module: "m".into(),
                outcome: Outcome::Passed,
                sim_time_steps: 10,
                wall_secs: 0.5,
                seed: 7,
                file: "t.rs".into(),
                line: 1,
            },
            TestResult {
                name: "b".into(),
                module: "m".into(),
                outcome: Outcome::Failed("bad \"thing\"\nline2".into()),
                sim_time_steps: 20,
                wall_secs: 0.25,
                seed: 8,
                file: "t.rs".into(),
                line: 1,
            },
            TestResult {
                name: "c".into(),
                module: "n".into(),
                outcome: Outcome::Skipped,
                sim_time_steps: 0,
                wall_secs: 0.0,
                seed: 0,
                file: "t.rs".into(),
                line: 1,
            },
        ];
        std::fs::write(&p, rivet_core::test::results_json(&results, "mock", -9)).unwrap();
        let (back, precision, _) = read_results_json(&p).unwrap();
        assert_eq!(precision, -9);
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].outcome, Outcome::Passed);
        assert_eq!(back[0].seed, 7);
        assert_eq!(back[1].outcome, Outcome::Failed("bad \"thing\"\nline2".into()));
        assert_eq!(back[2].outcome, Outcome::Skipped);
        assert_eq!(back[1].sim_time_steps, 20);
        assert!(read_results_json(&dir.join("missing.json")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sharding() {
        let names: Vec<String> = (0..5).map(|i| format!("t{i}")).collect();
        let s = shard(&names, 2);
        assert_eq!(s, vec![vec!["t0", "t2", "t4"], vec!["t1", "t3"]]);
        assert_eq!(shard(&names, 10).len(), 5, "no empty shards");
        assert_eq!(shard(&names, 0).len(), 1);
        assert_eq!(shard(&[], 3).len(), 0);
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
                "-j4",
                "--wall-timeout",
                "30",
                "--log-format",
                "json",
                "--param-set",
                "w16",
                "--cov-threshold",
                "90",
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
        assert_eq!(o.jobs, 4);
        assert_eq!(o.wall_timeout, Some(30.0));
        assert_eq!(o.log_format.as_deref(), Some("json"));
        assert_eq!(o.param_set.as_deref(), Some("w16"));
        assert_eq!(o.cov_threshold, Some(90.0));
        let o = parse_args(["cov", "report", "--threshold", "80", "a.json", "b.json"].map(String::from));
        assert_eq!((o.cmd.as_str(), o.sub.as_str()), ("cov", "report"));
        assert_eq!(o.cov_threshold, Some(80.0));
        assert_eq!(o.files, vec![PathBuf::from("a.json"), PathBuf::from("b.json")]);
        let o = parse_args(["run", "--jobs", "3", "--no-log-dir", "--update-golden"].map(String::from));
        assert_eq!(o.jobs, 3);
        assert!(o.no_log_dir && o.update_golden);
    }
}
