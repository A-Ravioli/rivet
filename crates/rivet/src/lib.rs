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
pub use rivet_macros::test;

#[cfg(feature = "verilator")]
pub use rivet_verilator as verilator;
#[cfg(feature = "vpi")]
pub use rivet_vpi as vpi;

/// Everything a testbench usually needs.
pub mod prelude {
    pub use crate::triggers::{
        first, join, next_time_step, read_only, read_write, with_timeout, yield_now, Either, Timer,
    };
    pub use crate::{
        bail, ensure, spawn, spawn_named, Clock, Event, JoinHandle, Lock, Logic, LogicVec, Module, Queue, Scope,
        Signal, TimeExt,
    };
    pub use log::{debug, error, info, trace, warn};
}
