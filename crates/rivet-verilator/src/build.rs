//! Build-script helper: run Verilator, compile the C++ shim, and emit link
//! directives. Use from a test crate's `build.rs`:
//!
//! ```ignore
//! fn main() {
//!     if std::env::var_os("CARGO_FEATURE_VERILATOR").is_some() {
//!         rivet_verilator::Build::new("dff").file("hdl/dff.sv").build();
//!     }
//! }
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Build {
    top: String,
    files: Vec<PathBuf>,
    include_dirs: Vec<PathBuf>,
    defines: Vec<(String, String)>,
    params: Vec<(String, String)>,
    args: Vec<String>,
    trace: bool,
    trace_fst: bool,
    timing: bool,
    verilator: String,
    warnings_fatal: bool,
}

impl Build {
    pub fn new(top: &str) -> Build {
        Build {
            top: top.to_string(),
            files: Vec::new(),
            include_dirs: Vec::new(),
            defines: Vec::new(),
            params: Vec::new(),
            args: Vec::new(),
            trace: false,
            trace_fst: false,
            timing: false,
            verilator: std::env::var("VERILATOR").unwrap_or_else(|_| "verilator".into()),
            warnings_fatal: false,
        }
    }

    /// Configure from the `rivet.toml` next to the crate's `Cargo.toml`
    /// (or an ancestor), including the `[sim.verilator]` section.
    pub fn from_manifest() -> Build {
        let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let m = rivet_manifest::Manifest::find(&dir).unwrap_or_else(|e| panic!("{e}"));
        println!("cargo:rerun-if-changed={}", m.dir.join("rivet.toml").display());
        let sim = m.sim("verilator");
        let mut b = Build::new(&m.design.top).files(m.sources_abs());
        for i in m.includes_abs() {
            b = b.include_dir(i);
        }
        for (k, v) in &m.design.defines {
            b = b.define(k, v);
        }
        for (k, v) in &m.design.params {
            b = b.param(k, v);
        }
        if let Some(ts) = &m.design.timescale {
            b = b.arg("--timescale").arg(ts);
        }
        for a in &sim.args {
            b = b.arg(a);
        }
        if sim.trace {
            b = b.trace();
        }
        if sim.timing {
            b = b.timing();
        }
        b
    }

    pub fn file(mut self, p: impl AsRef<Path>) -> Build {
        self.files.push(p.as_ref().to_path_buf());
        self
    }

    pub fn files<I: IntoIterator<Item = P>, P: AsRef<Path>>(mut self, ps: I) -> Build {
        for p in ps {
            self.files.push(p.as_ref().to_path_buf());
        }
        self
    }

    pub fn include_dir(mut self, p: impl AsRef<Path>) -> Build {
        self.include_dirs.push(p.as_ref().to_path_buf());
        self
    }

    pub fn define(mut self, k: &str, v: &str) -> Build {
        self.defines.push((k.into(), v.into()));
        self
    }

    /// Top-level parameter override (`-G`).
    pub fn param(mut self, k: &str, v: &str) -> Build {
        self.params.push((k.into(), v.into()));
        self
    }

    /// Extra Verilator argument.
    pub fn arg(mut self, a: &str) -> Build {
        self.args.push(a.into());
        self
    }

    /// Enable VCD tracing (`--trace`); the executable then accepts `--trace`.
    pub fn trace(mut self) -> Build {
        self.trace = true;
        self
    }

    /// Enable FST tracing (`--trace-fst`).
    pub fn trace_fst(mut self) -> Build {
        self.trace = true;
        self.trace_fst = true;
        self
    }

    /// Enable `--timing` for designs with delays/events.
    pub fn timing(mut self) -> Build {
        self.timing = true;
        self
    }

    pub fn warnings_fatal(mut self, v: bool) -> Build {
        self.warnings_fatal = v;
        self
    }

    /// Run Verilator and compile the shim. Panics with a readable message on
    /// failure, as build scripts do.
    pub fn build(self) {
        let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let obj_dir = out_dir.join("obj_dir");
        let prefix = format!("V{}", self.top);
        for f in &self.files {
            println!("cargo:rerun-if-changed={}", f.display());
        }
        println!("cargo:rerun-if-env-changed=VERILATOR");

        // Generated code is not portable across Verilator versions; start
        // from a clean obj_dir whenever the tool changes.
        let version = Command::new(&self.verilator)
            .arg("--version")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let stamp = out_dir.join("verilator.version");
        if std::fs::read_to_string(&stamp).ok().as_deref() != Some(version.as_str()) {
            let _ = std::fs::remove_dir_all(&obj_dir);
        }

        let mut cmd = Command::new(&self.verilator);
        cmd.args(["--cc", "--vpi", "--public-flat-rw", "--build", "-Mdir"])
            .arg(&obj_dir)
            .args(["--top-module", &self.top, "--prefix", &prefix])
            .args(["-CFLAGS", "-fPIC -std=gnu++17"]);
        // Verilator's default OPT_FAST is -Os; match Cargo's profile instead.
        let opt = if std::env::var("PROFILE").map(|p| p == "release").unwrap_or(false) { "-O2" } else { "-O1" };
        cmd.arg("-CFLAGS").arg(opt);
        if !self.warnings_fatal {
            cmd.arg("-Wno-fatal");
        }
        if self.trace {
            cmd.arg(if self.trace_fst { "--trace-fst" } else { "--trace" });
            cmd.arg("--trace-structs");
        }
        if self.timing {
            cmd.arg("--timing");
        }
        for d in &self.include_dirs {
            cmd.arg(format!("+incdir+{}", d.display()));
        }
        for (k, v) in &self.defines {
            cmd.arg(format!("+define+{k}={v}"));
        }
        for (k, v) in &self.params {
            cmd.arg(format!("-G{k}={v}"));
        }
        cmd.args(&self.args);
        cmd.args(&self.files);
        let out = cmd.output().unwrap_or_else(|e| panic!("cannot run {}: {e}", self.verilator));
        if !out.status.success() {
            panic!(
                "verilator failed ({}):\n{}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }

        std::fs::write(&stamp, &version).expect("write version stamp");

        let root = Command::new(&self.verilator)
            .args(["--getenv", "VERILATOR_ROOT"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .expect("verilator --getenv VERILATOR_ROOT");
        let include = PathBuf::from(&root).join("include");

        let shim = out_dir.join("rivet_shim.cpp");
        std::fs::write(&shim, shim_source(&prefix)).expect("write shim");
        let mut cc = cc::Build::new();
        cc.cpp(true)
            .file(&shim)
            .include(&include)
            .include(include.join("vltstd"))
            .include(&obj_dir)
            .flag_if_supported("-std=gnu++17")
            .flag_if_supported("-Wno-unused-parameter")
            .define("VL_TIME_CONTEXT", None)
            .warnings(false);
        if self.trace {
            cc.define("VM_TRACE", "1");
            if self.trace_fst {
                cc.define("VM_TRACE_FST", "1");
            }
        }
        if self.timing {
            cc.flag_if_supported("-fcoroutines");
        }
        cc.compile("rivet_shim");

        // rustc places a crate's own native libraries before the rlibs of
        // its dependencies, so `vpi_*` references from rivet-vpi would not
        // see libverilated.a. Append the archives as a link group instead.
        println!("cargo:rustc-link-search=native={}", obj_dir.display());
        println!("cargo:rustc-link-arg=-Wl,--start-group");
        println!("cargo:rustc-link-arg={}", out_dir.join("librivet_shim.a").display());
        println!("cargo:rustc-link-arg={}", obj_dir.join(format!("lib{prefix}.a")).display());
        println!("cargo:rustc-link-arg={}", obj_dir.join("libverilated.a").display());
        println!("cargo:rustc-link-arg=-Wl,--end-group");
        if self.trace_fst {
            println!("cargo:rustc-link-arg=-lz");
        }
        println!("cargo:rustc-link-arg=-lstdc++");
        println!("cargo:rustc-link-arg=-lpthread");
        println!("cargo:rustc-env=RIVET_VERILATOR_TOP={}", self.top);
    }
}

fn shim_source(prefix: &str) -> String {
    format!(
        r#"// Generated by rivet-verilator. Exposes the Verilated model to Rust.
#include "{prefix}.h"
#include "verilated.h"
#include "verilated_vpi.h"
#include <cstdint>
#include <cstdio>
#ifndef VM_TRACE
#define VM_TRACE 0
#endif
#ifndef VM_TRACE_FST
#define VM_TRACE_FST 0
#endif
#if VM_TRACE
#if VM_TRACE_FST
#include "verilated_fst_c.h"
typedef VerilatedFstC rivet_trace_t;
#else
#include "verilated_vcd_c.h"
typedef VerilatedVcdC rivet_trace_t;
#endif
#endif

extern "C" {{
void* rivet_vl_new(int argc, char** argv) {{
    Verilated::commandArgs(argc, argv);
    Verilated::fatalOnVpiError(false);
    return new {prefix}("");
}}
void rivet_vl_delete(void* p) {{ delete static_cast<{prefix}*>(p); }}
void rivet_vl_eval_step(void* p) {{ static_cast<{prefix}*>(p)->eval_step(); }}
void rivet_vl_eval_end_step(void* p) {{ static_cast<{prefix}*>(p)->eval_end_step(); }}
int rivet_vl_events_pending(void* p) {{ return static_cast<{prefix}*>(p)->eventsPending() ? 1 : 0; }}
uint64_t rivet_vl_next_time_slot(void* p) {{ return static_cast<{prefix}*>(p)->nextTimeSlot(); }}
void rivet_vl_final(void* p) {{ static_cast<{prefix}*>(p)->final(); }}
int rivet_vl_got_finish() {{ return Verilated::gotFinish() ? 1 : 0; }}
void rivet_vl_set_time(uint64_t t) {{ Verilated::threadContextp()->time(t); }}
uint64_t rivet_vl_time() {{ return Verilated::threadContextp()->time(); }}
int rivet_vl_time_precision() {{ return Verilated::threadContextp()->timeprecision(); }}
int rivet_vl_error_count() {{ return Verilated::threadContextp()->errorCount(); }}
int rivet_vl_vpi_call_cbs(uint32_t reason) {{ return VerilatedVpi::callCbs(reason) ? 1 : 0; }}
int rivet_vl_vpi_call_value_cbs() {{ return VerilatedVpi::callValueCbs() ? 1 : 0; }}
void rivet_vl_vpi_call_timed_cbs() {{ VerilatedVpi::callTimedCbs(); }}
uint64_t rivet_vl_vpi_next_deadline() {{ return VerilatedVpi::cbNextDeadline(); }}
int rivet_vl_trace_supported() {{ return VM_TRACE; }}
// Direct access to a public variable's storage (what --public-flat-rw
// registers for VPI). Returns 1 if found and packed-only (no unpacked dims).
int rivet_vl_var_find(const char* scope, const char* name, void** datap, int* vltype, int* width, int* is_param) {{
    const VerilatedScope* sp = Verilated::threadContextp()->scopeFind(scope);
    if (!sp) return 0;
    VerilatedVar* vp = sp->varFind(name);
    if (!vp) return 0;
    if (vp->udims() != 0) return 0;
    *datap = vp->datap();
    *vltype = static_cast<int>(vp->vltype());
#if defined(VERILATOR_VERSION_INTEGER) && VERILATOR_VERSION_INTEGER >= 5036000
    *width = vp->dims() == 0 ? 1 : vp->elements(0);
#else
    *width = vp->dims() == 0 ? 1 : vp->packed().elements();
#endif
    *is_param = vp->isParam() ? 1 : 0;
    return 1;
}}
void* rivet_vl_trace_open(void* p, const char* file) {{
#if VM_TRACE
    Verilated::traceEverOn(true);
    rivet_trace_t* t = new rivet_trace_t;
    static_cast<{prefix}*>(p)->trace(t, 99);
    t->open(file);
    return t;
#else
    (void)p; (void)file;
    return nullptr;
#endif
}}
void rivet_vl_trace_dump(void* t, uint64_t time) {{
#if VM_TRACE
    if (t) static_cast<rivet_trace_t*>(t)->dump(time);
#else
    (void)t; (void)time;
#endif
}}
void rivet_vl_trace_close(void* t) {{
#if VM_TRACE
    // Closed but deliberately not deleted; see cocotb verilator.cpp:66.
    if (t) static_cast<rivet_trace_t*>(t)->close();
#else
    (void)t;
#endif
}}
}}
"#
    )
}
