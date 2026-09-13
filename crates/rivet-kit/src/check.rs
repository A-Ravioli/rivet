//! Cycle-based checkers: the common SystemVerilog-assertion patterns as
//! async functions and background tasks, usable on every simulator.
//!
//! Sampling happens in the ReadOnly phase of each rising edge, so checkers
//! see settled values. The immediate checkers return at the start of the
//! following time step, so the caller may write signals right after them.
//! A background checker that fails records the failure against the running
//! test (like a panicking task) and stops.

use rivet_core::error::{Error, Result};
use rivet_core::handle::Signal;
use rivet_core::runtime;
use rivet_core::task::{spawn_named, JoinHandle};
use rivet_core::time::{format_time, Unit};
use rivet_core::triggers::{next_time_step, read_only};
use rivet_core::value::LogicVec;

fn now_str() -> String {
    format_time(runtime::now(), runtime::precision(), Unit::Ns)
}

/// `sig` keeps its value over the next `cycles` rising edges.
pub async fn assert_stable(clk: Signal, sig: Signal, cycles: u32) -> Result<()> {
    read_only().await;
    let first = sig.get();
    for i in 0..cycles {
        clk.rising_edge().await;
        read_only().await;
        let now = sig.get();
        if now != first {
            let msg =
                format!("{} changed from {first} to {now} at {} (cycle {} of {cycles})", sig.path(), now_str(), i + 1);
            next_time_step().await;
            return Err(Error::Msg(msg));
        }
    }
    next_time_step().await;
    Ok(())
}

/// `pred` holds at some rising edge within the next `cycles` edges
/// (checked at each edge, including the current time step's end).
pub async fn assert_within(clk: Signal, cycles: u32, mut pred: impl FnMut() -> bool) -> Result<()> {
    read_only().await;
    let mut ok = pred();
    let mut left = cycles;
    while !ok && left > 0 {
        clk.rising_edge().await;
        read_only().await;
        ok = pred();
        left -= 1;
    }
    let at = now_str();
    next_time_step().await;
    if ok {
        Ok(())
    } else {
        Err(Error::Msg(format!("condition did not hold within {cycles} cycles (at {at})")))
    }
}

/// `sig` becomes `value` within `cycles` edges.
pub async fn assert_becomes(clk: Signal, sig: Signal, value: u64, cycles: u32) -> Result<()> {
    assert_within(clk, cycles, || sig.get_u64_lossy() == value).await.map_err(|_| {
        Error::Msg(format!("{} did not become {value:#x} within {cycles} cycles (at {})", sig.path(), now_str()))
    })
}

/// Background checker: fails the test the first time `pred` is true at a
/// rising edge. Cancel the handle to stop checking.
pub fn assert_never(name: &str, clk: Signal, mut pred: impl FnMut() -> bool + 'static) -> JoinHandle<()> {
    let n = name.to_string();
    spawn_named(&format!("never({name})"), async move {
        loop {
            clk.rising_edge().await;
            read_only().await;
            if pred() {
                runtime::report_failure(format!("assert_never {n:?} violated at {}", now_str()));
                return;
            }
        }
    })
}

/// Background checker: fails the test the first time `pred` is false at a
/// rising edge.
pub fn assert_always(name: &str, clk: Signal, mut pred: impl FnMut() -> bool + 'static) -> JoinHandle<()> {
    let n = name.to_string();
    spawn_named(&format!("always({name})"), async move {
        loop {
            clk.rising_edge().await;
            read_only().await;
            if !pred() {
                runtime::report_failure(format!("assert_always {n:?} violated at {}", now_str()));
                return;
            }
        }
    })
}

/// Background checker: whenever `antecedent` is true at an edge,
/// `consequent` must be true at that edge or within the next `within`
/// edges (`|->` with a bounded delay). Every antecedent is tracked, so
/// overlapping windows each get their own deadline.
pub fn assert_implies(
    name: &str,
    clk: Signal,
    mut antecedent: impl FnMut() -> bool + 'static,
    mut consequent: impl FnMut() -> bool + 'static,
    within: u32,
) -> JoinHandle<()> {
    let n = name.to_string();
    spawn_named(&format!("implies({name})"), async move {
        // Remaining edges for each open obligation.
        let mut open: Vec<u32> = Vec::new();
        loop {
            clk.rising_edge().await;
            read_only().await;
            let c = consequent();
            if antecedent() {
                open.push(within);
            }
            if c {
                open.clear();
            } else {
                for o in open.iter_mut() {
                    if *o == 0 {
                        runtime::report_failure(format!(
                            "assert_implies {n:?}: consequent did not follow within {within} cycles (at {})",
                            now_str()
                        ));
                        return;
                    }
                    *o -= 1;
                }
            }
        }
    })
}

/// Background checker: fails the test if any of `signals` carries an X or Z
/// bit at a rising edge. Start it after reset is released.
pub fn assert_no_x(name: &str, clk: Signal, signals: Vec<Signal>) -> JoinHandle<()> {
    let n = name.to_string();
    spawn_named(&format!("no_x({name})"), async move {
        let mut buf = LogicVec::zeros(0);
        loop {
            clk.rising_edge().await;
            read_only().await;
            for s in &signals {
                s.read_into(&mut buf);
                if !buf.is_resolvable() {
                    runtime::report_failure(format!("{n}: {} is {buf} at {}", s.path(), now_str()));
                    return;
                }
            }
        }
    })
}

/// Sample `signals` every edge and report the first X/Z as an error
/// instead of failing the test (for designs where X is expected before a
/// point in time). Returns after `cycles` edges with the offending signal,
/// if any.
pub async fn find_x(clk: Signal, signals: &[Signal], cycles: u32) -> Option<(Signal, LogicVec, u64)> {
    let mut buf = LogicVec::zeros(0);
    let mut found = None;
    'outer: for _ in 0..cycles {
        clk.rising_edge().await;
        read_only().await;
        for s in signals {
            s.read_into(&mut buf);
            if !buf.is_resolvable() {
                found = Some((*s, buf.clone(), runtime::now()));
                break 'outer;
            }
        }
    }
    next_time_step().await;
    found
}
