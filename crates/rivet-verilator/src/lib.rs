//! Verilator backend: Rivet owns `main` and drives the Verilated model
//! directly, reproducing the region ordering of cocotb's `verilator.cpp`
//! (see `docs/design/00-cocotb-analysis.md` §3.5). Values and callbacks go
//! through Verilator's VPI shim via [`rivet_vpi`].

pub mod build;
pub mod sched;
pub use build::Build;

use rivet_core::runtime::{self, Event};
use sched::VerilatorBackend;

use rivet_vpi::ffi::{cbEndOfSimulation, cbStartOfSimulation};
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

extern "C" {
    fn rivet_vl_new(argc: c_int, argv: *mut *mut c_char) -> *mut c_void;
    fn rivet_vl_delete(p: *mut c_void);
    fn rivet_vl_eval_step(p: *mut c_void);
    fn rivet_vl_eval_end_step(p: *mut c_void);
    fn rivet_vl_events_pending(p: *mut c_void) -> c_int;
    fn rivet_vl_next_time_slot(p: *mut c_void) -> u64;
    fn rivet_vl_final(p: *mut c_void);
    fn rivet_vl_got_finish() -> c_int;
    fn rivet_vl_set_time(t: u64);
    fn rivet_vl_time() -> u64;
    fn rivet_vl_error_count() -> c_int;
    fn rivet_vl_vpi_call_cbs(reason: u32) -> c_int;
    fn rivet_vl_vpi_call_value_cbs() -> c_int;
    fn rivet_vl_vpi_call_timed_cbs();
    fn rivet_vl_vpi_next_deadline() -> u64;
    pub(crate) fn rivet_vl_trace_supported() -> c_int;
    fn rivet_vl_trace_fst() -> c_int;
    fn rivet_vl_trace_open(p: *mut c_void, file: *const c_char) -> *mut c_void;
    fn rivet_vl_trace_dump(t: *mut c_void, time: u64);
    fn rivet_vl_trace_close(t: *mut c_void);
}

const NO_DEADLINE: u64 = u64::MAX;

/// Options parsed from the executable's command line.
#[derive(Default, Debug)]
pub struct Options {
    pub trace: bool,
    pub trace_file: Option<String>,
    pub trace_flush: bool,
}

impl Options {
    pub fn from_args() -> Options {
        let mut o = Options::default();
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            match a.as_str() {
                "--trace" => o.trace = true,
                "--trace-flush" => o.trace_flush = true,
                "--trace-file" => o.trace_file = it.next(),
                "--help" | "-h" => {
                    eprintln!("usage: <sim> [--trace] [--trace-file FILE] [--trace-flush]\n\nrivet + Verilator simulation. Test selection: RIVET_TEST_FILTER=name,name; results: RIVET_RESULTS_FILE.");
                    std::process::exit(0);
                }
                _ => {}
            }
        }
        if o.trace_file.is_none() && std::env::var("RIVET_WAVES").map(|v| v == "1").unwrap_or(false) {
            o.trace = true;
        }
        o
    }
}

/// Settle value-change callbacks: they can write signals, so loop until
/// none fires.
unsafe fn settle_value_callbacks() -> bool {
    let mut any = false;
    while rivet_vl_vpi_call_value_cbs() != 0 {
        any = true;
    }
    any
}

/// Run the simulation to completion and return the process exit code.
pub fn run(opts: Options) -> i32 {
    // Verilated::commandArgs wants the real argv for plusargs.
    let args: Vec<CString> = std::env::args().map(|a| CString::new(a).unwrap()).collect();
    let mut argv: Vec<*mut c_char> = args.iter().map(|a| a.as_ptr() as *mut c_char).collect();
    argv.push(std::ptr::null_mut());
    unsafe {
        let top = rivet_vl_new(args.len() as c_int, argv.as_mut_ptr());
        let mut sched_slot = None;
        rivet_vpi::startup_with(|vpi| {
            let (b, s) = VerilatorBackend::new(vpi);
            sched_slot = Some(s);
            Box::new(b)
        });
        let sched = sched_slot.expect("scheduler");
        let mut trace: *mut c_void = std::ptr::null_mut();
        if opts.trace {
            if rivet_vl_trace_supported() == 0 {
                eprintln!("rivet: --trace requires the model to be built with Build::trace()");
                return 2;
            }
            let file = opts.trace_file.clone().unwrap_or_else(|| "dump.vcd".to_string());
            let mut s = sched.borrow_mut();
            s.wave_on = true;
            s.wave_file = Some(file);
            s.wave_dirty = true;
        }
        let ext = if rivet_vl_trace_fst() != 0 { "fst" } else { "vcd" };
        // (Re)open the dump file when the harness asked for a new one.
        let apply_waves = |trace: &mut *mut c_void| {
            let mut s = sched.borrow_mut();
            if !s.wave_dirty {
                return;
            }
            s.wave_dirty = false;
            if !trace.is_null() {
                rivet_vl_trace_dump(*trace, rivet_vl_time());
                rivet_vl_trace_close(*trace);
                *trace = std::ptr::null_mut();
            }
            if let Some(f) = s.wave_file.clone() {
                let file = if f.contains('.') { f } else { format!("{f}.{ext}") };
                log::info!("waveform file {file}");
                let cfile = CString::new(file).unwrap();
                *trace = rivet_vl_trace_open(top, cfile.as_ptr());
            }
        };
        apply_waves(&mut trace);

        rivet_vl_vpi_call_cbs(cbStartOfSimulation as u32);
        settle_value_callbacks();

        let finished = || rivet_vl_got_finish() != 0 || sched.borrow().finished;
        while !finished() {
            // Evaluation cycles until values settle, then ReadWrite; if the
            // ReadWrite callbacks wrote anything, evaluate again.
            loop {
                loop {
                    rivet_vl_eval_step(top);
                    if !settle_value_callbacks() {
                        break;
                    }
                }
                let rw = std::mem::take(&mut sched.borrow_mut().rw);
                if !rw {
                    break;
                }
                runtime::dispatch(Event::ReadWrite);
                let changed = settle_value_callbacks();
                if !changed && !sched.borrow().rw {
                    // Writes made in the ReadWrite phase went straight to the
                    // model (NoDelay on Verilator); one more eval settles them.
                    rivet_vl_eval_step(top);
                    if !settle_value_callbacks() {
                        break;
                    }
                }
                if finished() {
                    break;
                }
            }
            rivet_vl_eval_end_step(top);
            if std::mem::take(&mut sched.borrow_mut().ro) {
                runtime::dispatch(Event::ReadOnly);
            }
            apply_waves(&mut trace);
            if !trace.is_null() && sched.borrow().wave_on {
                rivet_vl_trace_dump(trace, rivet_vl_time());
            }
            if finished() {
                break;
            }
            // Jump to the next harness deadline, VPI deadline, or HDL event.
            let next_native = sched.borrow().next_deadline().unwrap_or(NO_DEADLINE);
            let next_cb = rivet_vl_vpi_next_deadline();
            let next_hdl = if rivet_vl_events_pending(top) != 0 { rivet_vl_next_time_slot(top) } else { NO_DEADLINE };
            let next = next_native.min(next_cb).min(next_hdl);
            if next == NO_DEADLINE {
                log::debug!("no pending callbacks or events; ending simulation");
                break;
            }
            rivet_vl_set_time(next);
            if std::mem::take(&mut sched.borrow_mut().nts) {
                runtime::dispatch(Event::NextTimeStep);
            }
            settle_value_callbacks();
            let due = sched.borrow_mut().take_due(next);
            for id in due {
                runtime::dispatch(Event::Timer(id));
            }
            rivet_vl_vpi_call_timed_cbs();
            settle_value_callbacks();
        }

        rivet_vl_final(top);
        if !trace.is_null() {
            rivet_vl_trace_dump(trace, rivet_vl_time());
            rivet_vl_trace_close(trace);
        }
        rivet_vl_vpi_call_cbs(cbEndOfSimulation as u32);
        let code = rivet_vpi::exit_code();
        let errors = rivet_vl_error_count();
        rivet_vl_delete(top);
        if code != 0 {
            code
        } else if errors > 0 {
            eprintln!("rivet: Verilator reported {errors} error(s)");
            1
        } else {
            0
        }
    }
}

/// Entry point for a test crate's `main.rs`:
/// `fn main() { rivet::verilator::main() }`.
pub fn main() -> ! {
    let opts = Options::from_args();
    std::process::exit(run(opts))
}
