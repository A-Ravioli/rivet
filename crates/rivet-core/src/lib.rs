//! Core of the Rivet hardware verification harness.
//!
//! See `docs/design/01-architecture.md` for the design. Users normally
//! depend on the `rivet` facade crate rather than this one.

pub mod backend;
pub mod clock;
pub mod composite;
pub mod coverage;
pub mod error;
pub mod executor;
pub mod fixture;
pub mod fxhash;
pub mod handle;
pub mod log;
pub mod random;
pub mod runtime;
pub mod sync;
pub mod task;
pub mod test;
pub mod time;
pub mod triggers;
pub mod value;
pub mod waves;

pub use backend::{Action, Backend, Capabilities, Handle, ObjInfo, ObjKind, OwnedValue, Value, WaveCmd};
pub use clock::Clock;
pub use coverage::{Bins, CoverPoint, Covergroup, Cross};
pub use error::{Error, Result};

/// End the current test as skipped rather than failed.
///
/// Use it when a test does not apply to the simulator or design in front of
/// it, for example a record member on a simulator whose interface does not
/// expose them:
///
/// ```ignore
/// if !dut.has_child("cmd") {
///     return Err(rivet::skip("this simulator does not expose record members"));
/// }
/// ```
pub fn skip(why: impl Into<String>) -> Error {
    Error::Skip(why.into())
}
pub use executor::WaitOn;
pub use handle::{Module, Object, Signal, Slice};
pub use random::{rng, Random, Randomize, Rng};
pub use runtime::{dump_tasks, Phase};
pub use sync::{Event, Lock, Queue};
pub use task::{spawn, spawn_named, JoinHandle, Scope};
pub use test::{Bind, TestDesc};
pub use time::{Duration, RoundMode, SimTime, TimeExt, Unit};
pub use triggers::{first, join, next_time_step, read_only, read_write, with_timeout, yield_now, Either, Timer};
pub use value::{IntoLogicVec, Logic, LogicVec, Unresolved};

/// Re-exported for the `#[rivet::test]` macro.
pub use inventory;

/// Current simulation time in precision steps.
pub fn now() -> u64 {
    runtime::now()
}

/// Current simulation time in the given unit.
pub fn now_in(unit: Unit) -> f64 {
    let steps = runtime::now();
    let prec = runtime::precision();
    let Some(exp) = unit.exponent() else { return steps as f64 };
    let scale = prec - exp;
    if scale >= 0 {
        steps as f64 * 10f64.powi(scale)
    } else {
        steps as f64 / 10f64.powi(-scale)
    }
}
