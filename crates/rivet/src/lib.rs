//! Rivet: a Rust hardware verification harness.
//!
//! ```ignore
//! use rivet::prelude::*;
//!
//! #[rivet::test(timeout = 10.us())]
//! async fn counts(dut: Module) -> rivet::Result<()> {
//!     let clk = dut.signal("clk")?;
//!     let _clock = Clock::start(clk, 10.ns());
//!     dut.signal("rst_n")?.set(0);
//!     clk.rising_edge().await;
//!     dut.signal("rst_n")?.set(1);
//!     for _ in 0..4 { clk.rising_edge().await; }
//!     read_only().await;
//!     assert_eq!(dut.signal("count")?.get_u64()?, 4);
//!     Ok(())
//! }
//! ```

pub use rivet_core::*;
pub use rivet_kit as kit;
pub use rivet_macros::test;
pub use rivet_macros::Randomize;

#[cfg(feature = "verilator")]
pub use rivet_verilator as verilator;

/// `cargo test` integration. In the test crate, add
///
/// ```toml
/// [[test]]
/// name = "sim"
/// harness = false
/// ```
///
/// and a `tests/sim.rs` containing
///
/// ```ignore
/// use example_dff as _;
/// fn main() -> std::process::ExitCode { rivet::harness::main() }
/// ```
///
/// Then `cargo test -p example-dff` runs the tests on `RIVET_SIM`
/// (default `icarus`); `cargo test -- --list` lists them without a
/// simulator.
#[cfg(feature = "harness")]
pub mod harness {
    pub fn main() -> std::process::ExitCode {
        let tests: Vec<(String, String)> =
            crate::test::all_tests().iter().map(|t| (t.module.to_string(), t.name.to_string())).collect();
        rivet_cli::harness_main(tests)
    }
}
#[cfg(feature = "vhpi")]
pub use rivet_vhpi as vhpi;
#[cfg(feature = "vpi")]
pub use rivet_vpi as vpi;

/// Everything a testbench usually needs.
pub mod prelude {
    pub use crate::triggers::{
        first, join, next_time_step, read_only, read_write, with_timeout, yield_now, Either, Timer,
    };
    pub use crate::{
        bail, ensure, rng, spawn, spawn_named, Bins, Clock, CoverPoint, Covergroup, Cross, Event, IntoLogicVec,
        JoinHandle, Lock, Logic, LogicVec, Module, Queue, Random, Randomize, Rng, Scope, Signal, TimeExt,
    };
    pub use log::{debug, error, info, trace, warn};
}
