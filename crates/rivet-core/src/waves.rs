//! Waveform control from the testbench.
//!
//! ```ignore
//! rivet::waves::start(Some("burst_case"));   // new file (Verilator) and dumping on
//! // ... the interesting window ...
//! rivet::waves::off();
//! ```
//!
//! What each simulator supports:
//!
//! | simulator | on/off | new file per call |
//! |---|---|---|
//! | Verilator | yes | yes (`<name>.vcd` or `.fst`) |
//! | Icarus (`--waves`) | yes (`$dumpon`/`$dumpoff`) | no: one file per run |
//! | GHDL | no (`--wave` covers the whole run) | no |
//!
//! With `rivet run --waves-per-test` the runner calls [`start`] with
//! `<module>__<test>` before each test and [`off`] after it.

use crate::backend::WaveCmd;
use crate::runtime;
use std::cell::Cell;

thread_local! {
    static WARNED: Cell<bool> = const { Cell::new(false) };
}

fn send(cmd: WaveCmd) -> bool {
    let what = format!("{cmd:?}");
    match runtime::with(|rt| rt.backend.waves(cmd)) {
        Ok(()) => true,
        Err(e) => {
            if !WARNED.replace(true) {
                log::warn!("waveform control {what} not available: {e}");
            }
            false
        }
    }
}

/// Start dumping. `file` names a new dump file on simulators that support
/// it (the extension is added by the backend); `None` continues the current
/// file. Returns `false` if the simulator cannot do it.
pub fn start(file: Option<&str>) -> bool {
    let mut ok = true;
    if let Some(f) = file {
        ok &= send(WaveCmd::File(f.to_string()));
    }
    send(WaveCmd::On) && ok
}

/// Resume dumping into the current file.
pub fn on() -> bool {
    send(WaveCmd::On)
}

/// Pause dumping.
pub fn off() -> bool {
    send(WaveCmd::Off)
}
