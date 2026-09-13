//! Wider semantic coverage of triggers, tasks, sync primitives, the signal
//! API, and determinism, against the mock simulator.

use rivet_core::backend::{Action, ObjKind, Value};
use rivet_core::runtime;
use rivet_core::triggers::{
    first, join, next_time_step, read_only, read_write, with_timeout, yield_now, Either, Timer,
};
use rivet_core::{spawn, spawn_named, Clock, Event, Lock, Logic, LogicVec, Phase, Queue, Scope, TimeExt};
use rivet_mock::{kernel_stats, run_test, Design};
use std::cell::RefCell;
use std::rc::Rc;

fn clocked() -> Design {
    let mut d = Design::new("top").precision(-9);
    let clk = d.logic("clk", 1);
    let din = d.logic("d", 8);
    let q = d.logic("q", 8);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.init(q, LogicVec::from_u64(8, 0));
    d.clock(clk, 5, 0);
    d.process(&[clk], move |s| {
        if s.rose(clk) {
            s.nba(q, s.get(din));
        }
    });
    d
}

// ---------------------------------------------------------------------------
// Triggers and phases

#[test]
fn next_time_step_and_phases() {
    run_test(clocked(), |dut| async move {
        let clk = dut.signal("clk")?;
        assert_eq!(runtime::phase(), Phase::BeginTimeStep, "before anything ran");
        clk.rising_edge().await;
        assert_eq!(runtime::phase(), Phase::ValuesChange);
        let t = rivet_core::now();
        read_write().await;
        assert_eq!(runtime::phase(), Phase::ValuesSettle);
        read_only().await;
        assert_eq!(runtime::phase(), Phase::EndTimeStep);
        // NextTimeStep and Timer are legal from ReadOnly; they move to the
        // beginning of a later time step.
        next_time_step().await;
        assert_eq!(runtime::phase(), Phase::BeginTimeStep);
        assert_eq!(rivet_core::now(), t + 5, "next time step is the falling edge");
        read_only().await;
        Timer::new(3.ns()).await;
        assert_eq!(runtime::phase(), Phase::BeginTimeStep);
        assert_eq!(rivet_core::now(), t + 8);
        Ok(())
    })
    .unwrap();
}

#[test]
fn timers_with_equal_deadlines_fire_in_registration_order() {
    let order = Rc::new(RefCell::new(Vec::new()));
    let o = order.clone();
    run_test(Design::new("top"), move |_| async move {
        let mut hs = Vec::new();
        for i in 0..4 {
            let o = o.clone();
            hs.push(spawn(async move {
                Timer::steps(7).await;
                o.borrow_mut().push(i);
            }));
        }
        for h in hs {
            h.await?;
        }
        assert_eq!(rivet_core::now(), 7);
        Ok(())
    })
    .unwrap();
    assert_eq!(*order.borrow(), vec![0, 1, 2, 3]);
}

#[test]
fn dropped_timer_is_removed_from_simulator() {
    run_test(Design::new("top"), |_| async move {
        match first(Timer::steps(50), Timer::steps(10)).await {
            Either::Right(()) => {}
            Either::Left(()) => panic!("shorter timer must win"),
        }
        assert_eq!(rivet_core::now(), 10);
        assert_eq!(runtime::debug_pending_timers(), 0, "loser timer deregistered from the runtime");
        assert_eq!(kernel_stats().0, 0, "and from the simulator");
        Ok(())
    })
    .unwrap();
}

#[test]
fn dropped_edge_waiter_is_deregistered() {
    run_test(clocked(), |dut| async move {
        let clk = dut.signal("clk")?;
        Timer::new(1.ns()).await; // past the t=0 edge; next rising edge is at 10ns
        match first(clk.rising_edge(), Timer::new(2.ns())).await {
            Either::Right(()) => {}
            Either::Left(()) => panic!("timer must win"),
        }
        assert_eq!(rivet_core::now(), 3);
        assert_eq!(runtime::debug_edge_waiters(clk.handle()), 0);
        // The simulator-side callback stays registered (persistent per signal).
        assert_eq!(kernel_stats().1, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn many_waiters_on_one_edge_wake_in_order() {
    let order = Rc::new(RefCell::new(Vec::new()));
    let o = order.clone();
    run_test(clocked(), move |dut| async move {
        let clk = dut.signal("clk")?;
        Timer::new(1.ns()).await; // past the t=0 edge
        let mut hs = Vec::new();
        for i in 0..5 {
            let o = o.clone();
            hs.push(spawn(async move {
                clk.rising_edge().await;
                o.borrow_mut().push((i, rivet_core::now()));
            }));
        }
        for h in hs {
            h.await?;
        }
        assert_eq!(runtime::debug_edge_waiters(clk.handle()), 0);
        assert_eq!(kernel_stats().1, 1, "one simulator callback shared by all waiters");
        Ok(())
    })
    .unwrap();
    let v = order.borrow();
    assert_eq!(v.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
    assert!(v.iter().all(|x| x.1 == 10), "all woke at the same edge: {v:?}");
}

#[test]
fn edge_kinds() {
    let mut d = Design::new("top").precision(-9);
    let a = d.logic("a", 1);
    let vec = d.logic("v", 4);
    d.init(vec, LogicVec::from_u64(4, 0));
    // a: X -> 0 at 1ns, 0 -> 1 at 2ns, 1 -> 0 at 3ns; v changes at 4ns and 5ns.
    d.at(1, move |s| s.set(a, LogicVec::from_u64(1, 0)));
    d.at(2, move |s| s.set(a, LogicVec::from_u64(1, 1)));
    d.at(3, move |s| s.set(a, LogicVec::from_u64(1, 0)));
    d.at(4, move |s| s.set(vec, LogicVec::from_u64(4, 6)));
    d.at(5, move |s| s.set(vec, LogicVec::from_u64(4, 7)));
    run_test(d, |dut| async move {
        let a = dut.signal("a")?;
        let v = dut.signal("v")?;
        assert_eq!(a.get().bit(0), Logic::X);
        a.falling_edge().await;
        assert_eq!(rivet_core::now(), 1, "X -> 0 counts as a falling edge, as on most simulators");
        a.rising_edge().await;
        assert_eq!(rivet_core::now(), 2);
        a.value_change().await;
        assert_eq!(rivet_core::now(), 3);
        v.value_change().await;
        assert_eq!(rivet_core::now(), 4);
        assert_eq!(v.get_u64()?, 6);
        // rising_edge on a vector looks at bit 0: 6 -> 7 sets it.
        v.rising_edge().await;
        assert_eq!(rivet_core::now(), 5);
        Ok(())
    })
    .unwrap();
}

#[test]
fn writes_in_read_write_cause_another_evaluation() {
    let mut d = Design::new("top").precision(-9);
    let a = d.logic("a", 8);
    let b = d.logic("b", 8);
    d.init(a, LogicVec::from_u64(8, 0));
    d.process(&[a], move |s| {
        let v = s.get_u64(a);
        s.set(b, LogicVec::from_u64(8, v + 1));
    });
    run_test(d, |dut| async move {
        let a = dut.signal("a")?;
        let b = dut.signal("b")?;
        Timer::steps(1).await;
        read_write().await;
        a.set(10); // in ReadWrite: goes straight to the simulator
        assert_eq!(a.get_u64()?, 0, "still inertial");
        read_write().await; // a second ReadWrite phase in the same time step
        assert_eq!(a.get_u64()?, 10);
        assert_eq!(b.get_u64()?, 11, "combinational logic reacted");
        assert_eq!(rivet_core::now(), 1, "no time passed");
        read_only().await;
        assert_eq!(rivet_core::now(), 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn trusted_inertial_mode_writes_through() {
    let mut d = Design::new("top").trust_inertial(true);
    d.logic("x", 4);
    run_test(d, |dut| async move {
        let x = dut.signal("x")?;
        x.set(3);
        assert_eq!(runtime::debug_pending_writes(), 0, "nothing buffered when the backend is trusted");
        assert_eq!(x.get().to_u64_lossy(), 0, "but still inertial in the simulator");
        read_write().await;
        assert_eq!(x.get_u64()?, 3);
        Ok(())
    })
    .unwrap();
}

#[test]
fn deposits_between_tests_are_discarded() {
    let mut d = Design::new("top");
    d.logic("x", 4);
    run_test(d, |dut| async move {
        let x = dut.signal("x")?;
        x.set(1);
        x.set(2);
        assert_eq!(runtime::debug_pending_writes(), 1, "same handle dedups");
        dut.signal("x")?.set_now(9);
        runtime::discard_pending_writes();
        assert_eq!(runtime::debug_pending_writes(), 0);
        read_only().await;
        assert_eq!(x.get_u64()?, 9, "discarded deposit never landed");
        Ok(())
    })
    .unwrap();
}

#[test]
fn force_blocks_deposits_and_hdl_until_release() {
    let mut d = Design::new("top");
    let a = d.logic("a", 4);
    let y = d.logic("y", 4);
    d.init(a, LogicVec::from_u64(4, 1));
    d.process(&[a], move |s| {
        let v = s.get(a);
        s.set(y, v);
    });
    run_test(d, |dut| async move {
        let a = dut.signal("a")?;
        let y = dut.signal("y")?;
        y.force(0xf);
        y.set(0); // deposit on a forced signal is ignored
        a.set(2);
        read_only().await;
        assert_eq!(y.get_u64()?, 0xf);
        Timer::steps(1).await;
        y.release();
        a.set(3);
        read_only().await;
        assert_eq!(y.get_u64()?, 3);
        // Explicit write() with an action.
        Timer::steps(1).await;
        let v = LogicVec::from_u64(4, 5);
        y.write(Value::Vec(&v), Action::Force)?;
        read_only().await;
        assert_eq!(y.get_u64()?, 5);
        Ok(())
    })
    .unwrap();
}

// ---------------------------------------------------------------------------
// Tasks

#[test]
fn scope_cancels_children_on_drop() {
    let ticks = Rc::new(RefCell::new(0));
    let t = ticks.clone();
    run_test(clocked(), move |dut| async move {
        let clk = dut.signal("clk")?;
        {
            let mut scope = Scope::new();
            let t2 = t.clone();
            let child = scope.spawn(async move {
                loop {
                    clk.rising_edge().await;
                    *t2.borrow_mut() += 1;
                }
            });
            Timer::new(31.ns()).await;
            assert!(child.is_alive());
            assert!(!child.is_done());
        }
        let seen = *t.borrow();
        assert!(seen >= 3, "child ran while the scope was alive: {seen}");
        Timer::new(50.ns()).await;
        assert_eq!(*t.borrow(), seen, "child stopped when the scope dropped");
        Ok(())
    })
    .unwrap();
}

#[test]
fn join_handle_edge_cases() {
    run_test(Design::new("top"), |_| async move {
        let h = spawn_named("quick", async { 7u8 });
        assert_eq!(h.name(), "quick");
        assert!(!h.is_done());
        yield_now().await;
        assert!(h.is_done());
        h.cancel(); // cancelling a finished task is a no-op
        assert_eq!(h.await?, 7);

        let h2 = spawn(async {
            Timer::steps(100).await;
        });
        h2.cancel();
        h2.cancel();
        assert!(matches!(h2.await, Err(rivet_core::Error::Cancelled)));
        assert_eq!(runtime::debug_pending_timers(), 0, "cancelled task's timer removed");

        // A detached task keeps running after its handle is dropped.
        let flag = Rc::new(RefCell::new(false));
        let f = flag.clone();
        let before = runtime::live_tasks();
        drop(spawn(async move {
            Timer::steps(5).await;
            *f.borrow_mut() = true;
        }));
        assert_eq!(runtime::live_tasks(), before + 1);
        Timer::steps(10).await;
        assert!(*flag.borrow());
        assert_eq!(runtime::live_tasks(), before, "detached task finished and was released");
        Ok(())
    })
    .unwrap();
}

#[test]
fn child_panic_is_reported_through_join_and_fails_the_test() {
    let r = run_test(Design::new("top"), |_| async move {
        let h = spawn(async {
            Timer::steps(1).await;
            panic!("child exploded");
        });
        let e = h.await.unwrap_err();
        assert!(matches!(e, rivet_core::Error::Panicked(ref m) if m.contains("child exploded")), "{e}");
        // The test itself is still marked failed even though we handled it.
        Timer::steps(1).await;
        Ok(())
    });
    assert!(r.unwrap_err().to_string().contains("child exploded"));
}

#[test]
fn yield_join_and_timeout_combinators() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    run_test(Design::new("top"), move |_| async move {
        let l2 = l.clone();
        let b = spawn(async move {
            l2.borrow_mut().push("b");
        });
        l.borrow_mut().push("a1");
        yield_now().await;
        l.borrow_mut().push("a2");
        b.await?;
        let (x, y) = join(
            async {
                Timer::steps(3).await;
                1
            },
            async {
                Timer::steps(5).await;
                2
            },
        )
        .await;
        assert_eq!((x, y), (1, 2));
        assert_eq!(rivet_core::now(), 5);
        let v = with_timeout(
            async {
                Timer::steps(1).await;
                42
            },
            10.ns(),
        )
        .await?;
        assert_eq!(v, 42);
        Ok(())
    })
    .unwrap();
    assert_eq!(*log.borrow(), vec!["a1", "b", "a2"]);
}

// ---------------------------------------------------------------------------
// Sync primitives

#[test]
fn queue_semantics() {
    run_test(Design::new("top"), |_| async move {
        let q: Queue<u32> = Queue::bounded(2);
        assert!(q.is_empty() && !q.is_full());
        assert!(q.try_put(1).is_ok());
        assert!(q.try_put(2).is_ok());
        assert!(q.is_full());
        assert_eq!(q.try_put(3), Err(3));
        assert_eq!(q.len(), 2);
        let q2 = q.clone();
        let producer = spawn(async move {
            q2.put(3).await; // blocks until a get
            rivet_core::now()
        });
        Timer::steps(4).await;
        assert_eq!(q.try_get(), Some(1));
        let t = producer.await?;
        assert_eq!(t, 4, "put completed once space appeared");
        assert_eq!(q.get().await, 2);
        assert_eq!(q.get().await, 3);
        assert_eq!(q.try_get(), None);
        let unbounded: Queue<u8> = Queue::new();
        for i in 0..100 {
            unbounded.try_put(i).unwrap();
        }
        assert_eq!(unbounded.len(), 100);
        Ok(())
    })
    .unwrap();
}

#[test]
fn lock_is_exclusive_and_fair() {
    let order = Rc::new(RefCell::new(Vec::new()));
    let o = order.clone();
    run_test(Design::new("top"), move |_| async move {
        let lock = Lock::new();
        let mut hs = Vec::new();
        for i in 0..3 {
            let lock = lock.clone();
            let o = o.clone();
            hs.push(spawn(async move {
                let _g = lock.acquire().await;
                o.borrow_mut().push((i, "in"));
                Timer::steps(2).await;
                o.borrow_mut().push((i, "out"));
            }));
        }
        for h in hs {
            h.await?;
        }
        assert!(!lock.is_locked());
        Ok(())
    })
    .unwrap();
    let v = order.borrow();
    assert_eq!(*v, vec![(0, "in"), (0, "out"), (1, "in"), (1, "out"), (2, "in"), (2, "out")]);
}

#[test]
fn event_set_clear_and_multiple_waiters() {
    run_test(Design::new("top"), |_| async move {
        let ev = Event::new();
        assert!(!ev.is_set());
        let mut hs = Vec::new();
        for _ in 0..3 {
            let ev = ev.clone();
            hs.push(spawn(async move {
                ev.wait().await;
                rivet_core::now()
            }));
        }
        Timer::steps(3).await;
        ev.set();
        for h in hs {
            assert_eq!(h.await?, 3);
        }
        ev.wait().await; // already set: immediate
        ev.clear();
        assert!(!ev.is_set());
        let r = with_timeout(ev.wait(), 2.ns()).await;
        assert!(r.is_err(), "cleared event blocks again");
        Ok(())
    })
    .unwrap();
}

// ---------------------------------------------------------------------------
// Clock

#[test]
fn clock_builder_options() {
    let mut d = Design::new("top").precision(-9);
    d.logic("c1", 1);
    d.logic("c2", 1);
    d.logic("c3", 1);
    run_test(d, |dut| async move {
        let c1 = dut.signal("c1")?;
        let c2 = dut.signal("c2")?;
        let c3 = dut.signal("c3")?;
        // Asymmetric duty cycle.
        let k1 = Clock::builder(c1, 10.ns()).high_time(3.ns()).start();
        assert_eq!(k1.period_steps(), 10);
        assert_eq!(k1.signal(), c1);
        c1.rising_edge().await;
        let t = rivet_core::now();
        c1.falling_edge().await;
        assert_eq!(rivet_core::now(), t + 3);
        c1.rising_edge().await;
        assert_eq!(rivet_core::now(), t + 10);
        // start_high(false): first transition is to 0, visible before any edge wait.
        let _k2 = Clock::builder(c2, 4.ns()).start_high(false).immediate().start();
        yield_now().await; // let the clock task run its first write
        assert_eq!(c2.get_u64()?, 0, "immediate write visible at once");
        c2.rising_edge().await;
        // cycles(n) and stop().
        let k3 = Clock::start(c3, 2.ns());
        let t0 = rivet_core::now();
        k3.cycles(5).await;
        assert!(rivet_core::now() - t0 >= 8);
        k3.stop();
        let r = with_timeout(c3.rising_edge(), 20.ns()).await;
        assert!(r.is_err(), "stopped clock produces no more edges");
        Ok(())
    })
    .unwrap();
}

// ---------------------------------------------------------------------------
// Signal and hierarchy API

#[test]
fn signal_kinds_and_accessors() {
    let mut d = Design::new("top");
    let sub = d.module(d.root(), "u_sub");
    let deep = d.module(sub, "u_deep");
    d.logic_in(deep, "leaf", 3);
    d.integer("n");
    d.real("r");
    d.param("P", 16, 0x1234);
    let vec = d.logic("v", 70);
    d.init(vec, LogicVec::from_u128(70, (1u128 << 69) | 5));
    run_test(d, |dut| async move {
        let n = dut.signal("n")?;
        assert_eq!(n.kind(), ObjKind::Integer);
        assert_eq!(n.get_i64()?, 0);
        n.set_int(-7);
        read_only().await;
        assert_eq!(n.get_i64()?, -7);
        assert_eq!(n.get_u64()? & 0xffff_ffff, 0xffff_fff9);
        Timer::steps(1).await;
        let r = dut.signal("r")?;
        assert_eq!(r.kind(), ObjKind::Real);
        r.set_real(2.5);
        read_only().await;
        assert!((r.get_real() - 2.5).abs() < 1e-12);
        let p = dut.signal("P")?;
        assert!(p.is_const());
        assert_eq!(p.get_u64()?, 0x1234);
        let v = dut.signal("v")?;
        assert_eq!(v.width(), 70);
        let mut buf = LogicVec::zeros(0);
        v.read_into(&mut buf);
        assert_eq!(buf.width(), 70);
        assert_eq!(buf.bit(69), Logic::One);
        assert_eq!(buf.to_u64_lossy(), 5, "lossy read drops the high bit");
        assert!(v.get_u64().is_err(), "does not fit in 64 bits");
        assert!(v.is_high().is_err());
        assert_eq!(v.get().to_u128()?, (1u128 << 69) | 5);
        let leaf = dut.path_signal("u_sub.u_deep.leaf")?;
        assert_eq!(leaf.path(), "top.u_sub.u_deep.leaf");
        assert_eq!(leaf.name(), "leaf");
        assert_eq!(leaf.info().type_name, "logicvec");
        assert!(leaf.get_u64().is_err(), "uninitialised logic is X");
        assert_eq!(leaf.get_u64_lossy(), 0);
        assert_eq!(format!("{leaf}"), "top.u_sub.u_deep.leaf");
        assert_eq!(format!("{leaf:?}"), "Signal(top.u_sub.u_deep.leaf)");
        // Hierarchy navigation.
        let sub = dut.module("u_sub")?;
        assert!(sub.has_child("u_deep") && !sub.has_child("nope"));
        assert_eq!(dut.path_module("u_sub.u_deep")?.name(), "u_deep");
        assert_eq!(dut.path_object("u_sub[0]")?.path(), "top.u_sub.u_deep", "index picks the nth child of a module");
        assert!(dut.path_object("u_sub[5]").is_err());
        assert!(dut.path_object("u_sub[x]").is_err());
        assert!(dut.path_object("u_sub[1").is_err());
        assert!(dut.module("n").is_err(), "a signal is not a module");
        assert!(dut.child("u_sub")?.as_signal().is_err(), "a module is not a signal");
        assert!(dut.object().as_module().is_ok());
        assert_eq!(dut.children()?.len(), 5);
        assert!(n.index(0).is_err() && n.member("x").is_err());
        assert_eq!(format!("{:?}", dut), "Module(top)");
        assert_eq!(format!("{:?}", dut.object()), "Object(top)");
        Ok(())
    })
    .unwrap();
}

#[test]
fn writing_a_constant_fails() {
    let mut d = Design::new("top");
    d.param("P", 8, 1);
    let r = run_test(d, |dut| async move {
        dut.signal("P")?.set(2);
        Ok(())
    });
    assert!(r.unwrap_err().to_string().contains("constant"));
}

#[test]
fn now_in_units() {
    run_test(Design::new("top").precision(-12), |_| async move {
        Timer::new(1.5.us()).await;
        assert_eq!(rivet_core::now(), 1_500_000);
        assert_eq!(rivet_core::now_in(rivet_core::Unit::Ns), 1500.0);
        assert_eq!(rivet_core::now_in(rivet_core::Unit::Us), 1.5);
        assert_eq!(rivet_core::now_in(rivet_core::Unit::Step), 1_500_000.0);
        Ok(())
    })
    .unwrap();
}

// ---------------------------------------------------------------------------
// Determinism

fn traffic_trace() -> Vec<(u64, u32)> {
    let trace = Rc::new(RefCell::new(Vec::new()));
    let t = trace.clone();
    run_test(clocked(), move |dut| async move {
        let clk = dut.signal("clk")?;
        let d = dut.signal("d")?;
        let mut hs = Vec::new();
        for i in 0..6u32 {
            let t = t.clone();
            hs.push(spawn(async move {
                for _ in 0..(3 + i) {
                    if i % 2 == 0 {
                        clk.rising_edge().await;
                    } else {
                        Timer::new((3 + i as u64).ns()).await;
                    }
                    t.borrow_mut().push((rivet_core::now(), i));
                    d.set(i as u64);
                }
            }));
        }
        for h in hs {
            h.await?;
        }
        Ok(())
    })
    .unwrap();
    let v = trace.borrow().clone();
    v
}

#[test]
fn identical_runs_produce_identical_traces() {
    let a = traffic_trace();
    let b = traffic_trace();
    assert_eq!(a, b);
    assert!(a.len() > 20);
}
