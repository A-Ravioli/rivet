//! `CompositeBackend`: one testbench over two procedural interfaces.
//!
//! A mixed-language design is one kernel seen through two interfaces, and
//! neither interface can see the other's objects. These tests stand two mock
//! backends in for that pair: the "verilog" half is the primary (it owns time
//! and the root), the "vhdl" half is a secondary reached by name across the
//! boundary. What they pin down is the routing, the handle and callback
//! tagging, and the cross-boundary lookup — not any real simulator's VPI/VHPI
//! coexistence, which needs a tool Rivet's CI does not have.

use rivet_core::backend::{Backend, CbKind, Handle, ObjKind};
use rivet_core::composite::{tag_handle, untag_handle, CompositeBackend, MAX_BACKENDS};
use rivet_core::runtime::{self, Event};
use rivet_core::triggers::read_write;
use rivet_core::{LogicVec, Module, TimeExt};
use rivet_mock::{Design, MockBackend, MockSim};
use std::cell::RefCell;
use std::rc::Rc;

/// The Verilog half: the clock, and a counter of its own.
fn verilog_half() -> Design {
    let mut d = Design::new("top").precision(-9);
    let clk = d.logic("clk", 1);
    d.init(clk, LogicVec::from_u64(1, 0));
    d.clock(clk, 5, 0);
    let core = d.module(d.root(), "v_core");
    let count = d.logic_in(core, "count", 8);
    d.init(count, LogicVec::from_u64(8, 0));
    d.process(&[clk], move |s| {
        if s.rose(clk) {
            let n = s.get_u64(count).wrapping_add(1);
            s.nba(count, LogicVec::from_u64(8, n));
        }
    });
    d
}

/// The VHDL half: the same top, a region the primary cannot see, and an
/// accumulator driven by a signal the testbench writes.
fn vhdl_half() -> Design {
    let mut d = Design::new("top").precision(-9);
    let core = d.module(d.root(), "u_vhdl");
    let enable = d.logic_in(core, "enable", 1);
    let acc = d.logic_in(core, "acc", 8);
    d.init(enable, LogicVec::from_u64(1, 0));
    d.init(acc, LogicVec::from_u64(8, 0));
    d.process(&[enable], move |s| {
        if s.get_u64(enable) == 1 {
            let n = s.get_u64(acc).wrapping_add(1);
            s.set(acc, LogicVec::from_u64(8, n));
        }
    });
    d
}

fn compose() -> (CompositeBackend, MockSim, MockSim) {
    let primary = verilog_half().into_backend();
    let secondary = vhdl_half().into_backend();
    let psim = primary.driver();
    let ssim = secondary.driver();
    let c = CompositeBackend::new(Box::new(primary), vec![Box::new(secondary)]).expect("compose");
    (c, psim, ssim)
}

#[test]
fn identity_and_capabilities_come_from_both_halves() {
    let (c, _p, _s) = compose();
    assert_eq!(c.len(), 2);
    assert_eq!(c.name(), "mock+mock");
    assert!(c.version().contains(" + "), "{}", c.version());
    assert_eq!(c.precision(), -9);
    let caps = c.caps();
    // Neither mock trusts inertial writes, and both are four-state, so the
    // conservative intersection is what the runtime sees.
    assert!(!caps.trusts_inertial_writes);
    assert!(caps.four_state && caps.supports_force);
}

#[test]
fn a_secondary_that_cannot_tag_its_events_is_refused() {
    struct Untaggable(MockBackend);
    // Everything else delegates; `set_event_tag` keeps the trait default,
    // which refuses.
    impl Backend for Untaggable {
        fn name(&self) -> &str {
            "untaggable"
        }
        fn version(&self) -> String {
            self.0.version()
        }
        fn caps(&self) -> rivet_core::Capabilities {
            self.0.caps()
        }
        fn precision(&self) -> i32 {
            self.0.precision()
        }
        fn now(&self) -> u64 {
            self.0.now()
        }
        fn root(&mut self, n: Option<&str>) -> rivet_core::backend::Result<Handle> {
            self.0.root(n)
        }
        fn child_by_name(&mut self, p: Handle, n: &str) -> rivet_core::backend::Result<Option<Handle>> {
            self.0.child_by_name(p, n)
        }
        fn child_by_index(&mut self, p: Handle, i: i64) -> rivet_core::backend::Result<Option<Handle>> {
            self.0.child_by_index(p, i)
        }
        fn children(&mut self, p: Handle) -> rivet_core::backend::Result<Vec<Handle>> {
            self.0.children(p)
        }
        fn info(&self, h: Handle) -> &rivet_core::ObjInfo {
            self.0.info(h)
        }
        fn read(&mut self, h: Handle) -> rivet_core::backend::Result<rivet_core::OwnedValue> {
            self.0.read(h)
        }
        fn read_vec(&mut self, h: Handle, o: &mut LogicVec) -> rivet_core::backend::Result<()> {
            self.0.read_vec(h, o)
        }
        fn write(
            &mut self,
            h: Handle,
            v: rivet_core::Value<'_>,
            a: rivet_core::Action,
        ) -> rivet_core::backend::Result<()> {
            self.0.write(h, v, a)
        }
        fn register(&mut self, k: CbKind) -> rivet_core::backend::Result<rivet_core::backend::CbId> {
            self.0.register(k)
        }
        fn remove(&mut self, id: rivet_core::backend::CbId) -> rivet_core::backend::Result<()> {
            self.0.remove(id)
        }
        fn finish(&mut self) {
            self.0.finish()
        }
    }
    let primary = verilog_half().into_backend();
    let secondary = Untaggable(vhdl_half().into_backend());
    let e = CompositeBackend::new(Box::new(primary), vec![Box::new(secondary)]).unwrap_err();
    assert!(e.to_string().contains("event tagging on untaggable"), "{e}");
}

#[test]
fn halves_that_disagree_about_precision_are_refused() {
    let primary = verilog_half().into_backend();
    let secondary = vhdl_half().precision(-12).into_backend();
    let e = CompositeBackend::new(Box::new(primary), vec![Box::new(secondary)]).unwrap_err();
    assert!(e.to_string().contains("precision"), "{e}");
    assert!(e.to_string().contains("one kernel"), "{e}");
}

#[test]
fn too_many_backends_are_refused() {
    let primary = verilog_half().into_backend();
    let secondaries: Vec<Box<dyn Backend>> =
        (0..MAX_BACKENDS).map(|_| Box::new(vhdl_half().into_backend()) as Box<dyn Backend>).collect();
    let e = CompositeBackend::new(Box::new(primary), secondaries).unwrap_err();
    assert!(e.to_string().contains("handle tag holds"), "{e}");
}

#[test]
fn lookups_cross_the_language_boundary_and_stay_tagged() {
    let (mut c, _p, _s) = compose();
    let root = c.root(None).unwrap();
    assert_eq!(untag_handle(root).0, 0, "the primary owns the root");
    assert_eq!(c.info(root).path, "top");

    // The primary's own child: found directly, tag 0.
    let v = c.child_by_name(root, "v_core").unwrap().expect("v_core");
    assert_eq!(untag_handle(v).0, 0);
    assert_eq!(c.info(v).path, "top.v_core");

    // A region only the other interface can see: found by full path from the
    // secondary's own root, and carries its tag from here on.
    let u = c.child_by_name(root, "u_vhdl").unwrap().expect("u_vhdl");
    assert_eq!(untag_handle(u).0, 1, "the secondary owns u_vhdl");
    assert_eq!(c.info(u).kind, ObjKind::Module);
    let acc = c.child_by_name(u, "acc").unwrap().expect("acc");
    assert_eq!(untag_handle(acc).0, 1);
    assert_eq!(c.info(acc).path, "top.u_vhdl.acc");
    assert_eq!(c.info(acc).width, 8);

    // Children of a secondary region are all tagged.
    let kids = c.children(u).unwrap();
    assert_eq!(kids.len(), 2);
    assert!(kids.iter().all(|h| untag_handle(*h).0 == 1));
    assert_eq!(c.child_by_index(u, 0).unwrap(), Some(kids[0]));

    // A name in neither half is simply absent, not an error.
    assert_eq!(c.child_by_name(root, "nope").unwrap(), None);
}

#[test]
fn reads_writes_and_callbacks_route_to_the_owning_half() {
    let (mut c, _p, mut s) = compose();
    let root = c.root(None).unwrap();
    let u = c.child_by_name(root, "u_vhdl").unwrap().unwrap();
    let acc = c.child_by_name(u, "acc").unwrap().unwrap();
    let enable = c.child_by_name(u, "enable").unwrap().unwrap();
    let count = {
        let v = c.child_by_name(root, "v_core").unwrap().unwrap();
        c.child_by_name(v, "count").unwrap().unwrap()
    };

    // A write to the secondary reaches the secondary's kernel, and only it.
    c.write(enable, rivet_core::Value::Int(1), rivet_core::Action::NoDelay).unwrap();
    s.eval();
    assert_eq!(c.read(acc).unwrap(), rivet_core::OwnedValue::Vec(LogicVec::from_u64(8, 1)));
    assert_eq!(c.read(count).unwrap(), rivet_core::OwnedValue::Vec(LogicVec::from_u64(8, 0)));

    // A value-change callback goes to the interface that owns the object,
    // and its id carries that interface's tag; a timer goes to the primary,
    // which owns the one time wheel.
    let vc = c.register(CbKind::ValueChange(acc)).unwrap();
    assert_eq!(rivet_core::composite::untag_cb(vc).0, 1);
    let t = c.register(CbKind::AfterDelay(10)).unwrap();
    assert_eq!(rivet_core::composite::untag_cb(t).0, 0);
    c.remove(vc).unwrap();
    c.remove(t).unwrap();

    // A handle carrying a tag no backend answers to is an error, not a panic.
    let bogus = tag_handle(5, Handle(0));
    assert!(c.read(bogus).is_err());
}

/// The end-to-end shape: a test body drives the clock in one half and the
/// signals of the other, through one `Module` tree.
#[test]
fn a_testbench_drives_both_halves_through_one_hierarchy() {
    rivet_core::log::init();
    let primary = verilog_half().into_backend();
    let secondary = vhdl_half().into_backend();
    let mut psim = primary.driver();
    let ssim = Rc::new(RefCell::new(secondary.driver()));
    let c = CompositeBackend::new(Box::new(primary), vec![Box::new(secondary)]).unwrap();
    runtime::init(Box::new(c));

    type Counts = Rc<RefCell<Option<rivet_core::Result<(u64, u64)>>>>;
    let out: Counts = Rc::new(RefCell::new(None));
    let slot = out.clone();
    let pump = ssim.clone();
    runtime::set_entry(move || {
        let root = runtime::backend(|b| b.root(None)).expect("root");
        runtime::set_root(root);
        rivet_core::spawn_named("test", async move {
            let body = async {
                let dut = Module::from_handle(root);
                let clk = dut.signal("clk")?;
                let count = dut.module("v_core")?.signal("count")?;
                let vhdl = dut.module("u_vhdl")?;
                let enable = vhdl.signal("enable")?;
                let acc = vhdl.signal("acc")?;
                for _ in 0..4 {
                    clk.rising_edge().await;
                    // Toggle the other language's input, let the write land,
                    // then settle that kernel: a real mixed-language
                    // simulator settles both halves itself.
                    enable.set(1u64);
                    read_write().await;
                    pump.borrow_mut().eval();
                    enable.set(0u64);
                    read_write().await;
                    pump.borrow_mut().eval();
                }
                rivet_core::triggers::Timer::new(1.ns()).await;
                Ok((count.get().to_u64().unwrap(), acc.get().to_u64().unwrap()))
            };
            let r = body.await;
            *slot.borrow_mut() = Some(r);
            runtime::finish();
        });
    });
    psim.run();
    runtime::shutdown();

    let (count, acc) = out.borrow_mut().take().expect("test ran").expect("test passed");
    assert_eq!(count, 4, "the primary half counted its own clock");
    assert_eq!(acc, 4, "the secondary half saw every write routed to it");
}

#[test]
fn a_secondary_tags_the_events_it_dispatches() {
    // `set_event_tag` is what makes a secondary's callbacks distinguishable
    // once they reach the runtime; the mock applies it at dispatch.
    let mut b = vhdl_half().into_backend();
    b.set_event_tag(3).unwrap();
    assert_eq!(
        rivet_core::composite::tag_event(3, Event::ValueChange(Handle(2))),
        Event::ValueChange(tag_handle(3, Handle(2)))
    );
}
