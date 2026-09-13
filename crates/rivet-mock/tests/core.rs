//! Executor, trigger, and timing-model tests against the mock simulator.

use rivet_core::triggers::{first, read_only, read_write, with_timeout, Either, Timer};
use rivet_core::{spawn, Clock, Event, Lock, LogicVec, Queue, TimeExt};
use rivet_mock::{run_test, Design};
use std::cell::RefCell;
use std::rc::Rc;

fn dff() -> Design {
    let mut d = Design::new("top");
    let clk = d.logic("clk", 1);
    let din = d.logic("d", 8);
    let q = d.logic("q", 8);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.init(q, LogicVec::from_u64(8, 0));
    d.process(&[clk], move |s| {
        if s.rose(clk) {
            s.nba(q, s.get(din));
        }
    });
    d
}

#[test]
fn dff_captures_on_rising_edge() {
    run_test(dff(), |dut| async move {
        let clk = dut.signal("clk")?;
        let d = dut.signal("d")?;
        let q = dut.signal("q")?;
        let _clock = Clock::start(clk, 10.ns());
        d.set(0x5a);
        clk.rising_edge().await;
        // Values-change phase: q has not updated yet.
        assert_eq!(q.get_u64()?, 0);
        read_write().await;
        assert_eq!(q.get_u64()?, 0x5a, "q updates by the ReadWrite phase");
        d.set(0xa5);
        clk.rising_edge().await;
        read_only().await;
        assert_eq!(q.get_u64()?, 0xa5);
        Ok(())
    })
    .unwrap();
}

#[test]
fn timer_advances_time() {
    run_test(Design::new("top").precision(-9), |_dut| async move {
        assert_eq!(rivet_core::now(), 0);
        Timer::new(10.ns()).await;
        assert_eq!(rivet_core::now(), 10);
        Timer::new(2.5.us()).await;
        assert_eq!(rivet_core::now(), 2510);
        assert_eq!(rivet_core::now_in(rivet_core::Unit::Us), 2.51);
        Ok(())
    })
    .unwrap();
}

#[test]
fn deposit_is_inertial_and_visible_at_read_write() {
    let mut d = Design::new("top");
    d.logic("x", 4);
    run_test(d, |dut| async move {
        let x = dut.signal("x")?;
        x.set_now(1);
        assert_eq!(x.get_u64()?, 1, "NoDelay writes are visible immediately");
        x.set(7);
        assert_eq!(x.get_u64()?, 1, "Deposit is not visible until the simulator evaluates");
        read_write().await;
        // Buffered deposits are handed to the simulator at the start of the
        // ReadWrite phase as inertial writes; they are not readable back in
        // this phase (cocotb's test_read_back_in_readwrite is expect_fail on
        // simulators that do not trust inertial writes).
        read_only().await;
        assert_eq!(x.get_u64()?, 7);
        // Latest write to a handle in a timestep wins.
        Timer::steps(1).await;
        x.set(2);
        x.set(3);
        read_only().await;
        assert_eq!(x.get_u64()?, 3);
        // A write made in the ReadWrite phase is applied before ReadOnly.
        Timer::steps(1).await;
        read_write().await;
        x.set(4);
        read_only().await;
        assert_eq!(x.get_u64()?, 4);
        Ok(())
    })
    .unwrap();
}

#[test]
fn read_only_forbids_writes_and_illegal_transitions() {
    let mut d = Design::new("top");
    d.logic("x", 1);
    let r = run_test(d, |dut| async move {
        let x = dut.signal("x")?;
        Timer::steps(1).await;
        read_only().await;
        assert_eq!(rivet_core::runtime::phase(), rivet_core::Phase::EndTimeStep);
        x.set(1); // must panic
        Ok(())
    });
    let msg = r.unwrap_err().to_string();
    assert!(msg.contains("ReadOnly"), "{msg}");

    let mut d = Design::new("top");
    d.logic("x", 1);
    let r = run_test(d, |_dut| async move {
        Timer::steps(1).await;
        read_only().await;
        read_write().await; // illegal
        Ok(())
    });
    assert!(r.unwrap_err().to_string().contains("illegal transition"));
}

#[test]
fn cancelled_task_stops_waiting() {
    let counter = Rc::new(RefCell::new(0));
    let c2 = counter.clone();
    run_test(dff(), move |dut| async move {
        let clk = dut.signal("clk")?;
        let _clock = Clock::start(clk, 10.ns());
        let c = c2.clone();
        let h = spawn(async move {
            loop {
                clk.rising_edge().await;
                *c.borrow_mut() += 1;
            }
        });
        clk.rising_edge().await;
        clk.rising_edge().await;
        h.cancel();
        assert!(h.is_done());
        let seen = *c2.borrow();
        Timer::new(100.ns()).await;
        assert_eq!(*c2.borrow(), seen, "cancelled task must not run again");
        let r = h.await;
        assert!(matches!(r, Err(rivet_core::Error::Cancelled)));
        Ok(())
    })
    .unwrap();
    assert!(*counter.borrow() >= 1);
}

#[test]
fn join_handle_returns_value_and_panic() {
    run_test(Design::new("top"), |_dut| async move {
        let h = spawn(async {
            Timer::steps(5).await;
            42u32
        });
        assert_eq!(h.await?, 42);
        Ok(())
    })
    .unwrap();

    let r = run_test(Design::new("top"), |_dut| async move {
        let h = spawn(async {
            Timer::steps(1).await;
            panic!("boom");
        });
        // Even if we never observe the JoinHandle, the panic fails the test.
        drop(h);
        Timer::steps(10).await;
        Ok(())
    });
    let msg = r.unwrap_err().to_string();
    assert!(msg.contains("boom"), "{msg}");
}

#[test]
fn fifo_scheduling_is_deterministic() {
    let order = Rc::new(RefCell::new(Vec::new()));
    let o = order.clone();
    run_test(Design::new("top"), move |_dut| async move {
        let mut handles = Vec::new();
        for i in 0..5 {
            let o = o.clone();
            handles.push(spawn(async move {
                o.borrow_mut().push(i);
                Timer::steps(1).await;
                o.borrow_mut().push(i + 10);
            }));
        }
        for h in handles {
            h.await?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(*order.borrow(), vec![0, 1, 2, 3, 4, 10, 11, 12, 13, 14]);
}

#[test]
fn first_and_timeout() {
    run_test(dff(), |dut| async move {
        let clk = dut.signal("clk")?;
        let _clock = Clock::start(clk, 10.ns());
        clk.rising_edge().await; // first edge is at t=0
        match first(clk.rising_edge(), Timer::new(1.ns())).await {
            Either::Left(()) => panic!("timer should win"),
            Either::Right(()) => {}
        }
        assert_eq!(rivet_core::now(), 1000);
        with_timeout(clk.rising_edge(), 100.ns()).await?;
        let r = with_timeout(std::future::pending::<()>(), 3.ns()).await;
        assert!(matches!(r, Err(rivet_core::Error::Timeout(_))));
        Ok(())
    })
    .unwrap();
}

#[test]
fn event_queue_lock() {
    run_test(Design::new("top"), |_dut| async move {
        let ev = Event::new();
        let q: Queue<u32> = Queue::bounded(2);
        let lock = Lock::new();
        let ev2 = ev.clone();
        let q2 = q.clone();
        let lock2 = lock.clone();
        let producer = spawn(async move {
            let _g = lock2.acquire().await;
            for i in 0..5 {
                q2.put(i).await;
                Timer::steps(1).await;
            }
            ev2.set();
        });
        let mut got = Vec::new();
        while got.len() < 5 {
            got.push(q.get().await);
        }
        ev.wait().await;
        producer.await?;
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
        assert!(!lock.is_locked());
        Ok(())
    })
    .unwrap();
}

#[test]
fn hdl_clock_and_combinational_logic() {
    let mut d = Design::new("top").precision(-9);
    let clk = d.logic("clk", 1);
    let a = d.logic("a", 8);
    let b = d.logic("b", 8);
    let sum = d.logic("sum", 9);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.clock(clk, 5, 0);
    d.process(&[a, b], move |s| {
        let v = s.get_u64(a) + s.get_u64(b);
        s.set(sum, LogicVec::from_u64(9, v));
    });
    run_test(d, |dut| async move {
        let clk = dut.signal("clk")?;
        let a = dut.signal("a")?;
        let b = dut.signal("b")?;
        let sum = dut.signal("sum")?;
        for i in 0..20u64 {
            clk.rising_edge().await;
            a.set(i);
            b.set(i * 3);
            read_only().await;
            assert_eq!(sum.get_u64()?, i + i * 3, "cycle {i}");
        }
        assert_eq!(rivet_core::now(), 190);
        Ok(())
    })
    .unwrap();
}

#[test]
fn force_and_release() {
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
        read_write().await;
        assert_eq!(y.get_u64()?, 1);
        y.force(9);
        a.set(2);
        read_write().await;
        assert_eq!(y.get_u64()?, 9, "forced value overrides the driver");
        y.release();
        a.set(3);
        read_write().await;
        assert_eq!(y.get_u64()?, 3);
        Ok(())
    })
    .unwrap();
}

#[test]
fn value_change_and_falling_edge() {
    let mut d = Design::new("top").precision(-9);
    let clk = d.logic("clk", 1);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.clock(clk, 5, 0);
    run_test(d, |dut| async move {
        let clk = dut.signal("clk")?;
        clk.rising_edge().await;
        let t0 = rivet_core::now();
        clk.falling_edge().await;
        assert_eq!(rivet_core::now(), t0 + 5);
        clk.value_change().await;
        assert_eq!(rivet_core::now(), t0 + 10);
        assert!(clk.is_high()?);
        Ok(())
    })
    .unwrap();
}

#[test]
fn hierarchy_paths() {
    let mut d = Design::new("top");
    let sub = d.module(d.root(), "u_sub");
    let x = d.logic_in(sub, "x", 16);
    d.init(x, LogicVec::from_u64(16, 0xbeef));
    d.param("WIDTH", 32, 16);
    run_test(d, |dut| async move {
        assert_eq!(dut.path_signal("u_sub.x")?.get_u64()?, 0xbeef);
        assert_eq!(dut.module("u_sub")?.signal("x")?.path(), "top.u_sub.x");
        assert_eq!(dut.signal("WIDTH")?.get_u64()?, 16);
        assert!(dut.signal("WIDTH")?.is_const());
        assert!(dut.signal("nope").is_err());
        assert!(dut.module("u_sub")?.signal("x")?.width() == 16);
        Ok(())
    })
    .unwrap();
}

#[test]
fn slices_read_and_write_bit_ranges() {
    let mut d = Design::new("top").precision(-9);
    let x = d.logic("x", 16);
    d.init(x, LogicVec::from_u64(16, 0xabcd));
    run_test(d, |dut| async move {
        let x = dut.signal("x")?;
        // Reading a range.
        assert_eq!(x.slice(15, 12).get_u64()?, 0xa);
        assert_eq!(x.slice(7, 0).get_u64()?, 0xcd);
        assert_eq!(x.slice(3, 3).get_u64()?, 1);
        assert_eq!(x.slice(7, 0).width(), 8);
        assert_eq!(x.slice(7, 4).path(), "top.x[7:4]");

        // Writing a range leaves the rest alone, and two writes in one
        // phase compose instead of the second dropping the first.
        x.slice(7, 0).set(0x12);
        x.slice(15, 8).set(0x34);
        Timer::steps(1).await;
        assert_eq!(x.get_u64()?, 0x3412);

        // A slice write after a whole-signal write in the same phase starts
        // from the buffered value.
        x.set(0xffffu64);
        x.slice(11, 8).set(0x0);
        Timer::steps(1).await;
        assert_eq!(x.get_u64()?, 0xf0ff);
        Ok(())
    })
    .unwrap();
}
