//! A pure-Rust event simulator implementing [`rivet_core::Backend`].
//!
//! It exists so the executor, triggers, write buffering, and test runner
//! can be tested without an EDA tool, and so testbenches can run against
//! Rust behavioural models. The scheduling loop mirrors the one cocotb
//! supplies for Verilator (`verilator.cpp`): evaluate to a fixpoint,
//! ReadWrite, evaluate again if anything changed, ReadOnly, jump to the next
//! timed event, NextTimeStep, timed callbacks.
//!
//! ```ignore
//! let mut d = Design::new("top");
//! let clk = d.logic("clk", 1);
//! let din = d.logic("d", 8);
//! let q = d.logic("q", 8);
//! d.process(&[clk], move |s| if s.rose(clk) { s.nba(q, s.get(din)) });
//! ```

use rivet_core::backend::*;
use rivet_core::runtime::{self, Event};
use rivet_core::value::{Logic, LogicVec};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

/// Identifier of a signal or module in a [`Design`].
pub type Id = Handle;

struct Node {
    info: ObjInfo,
    children: Vec<Handle>,
    by_name: HashMap<String, Handle>,
    // value objects only
    value: LogicVec,
    prev: LogicVec,
    forced: Option<LogicVec>,
    real: f64,
    string: String,
}

type ProcBody = Box<dyn FnMut(&mut ProcCtx<'_>)>;

struct Proc {
    sens: Vec<Handle>,
    body: ProcBody,
}

type EventBody = Box<dyn FnOnce(&mut ProcCtx<'_>)>;

/// The design under construction: hierarchy, signals, processes, and
/// scheduled events.
pub struct Design {
    nodes: Vec<Node>,
    procs: Vec<Proc>,
    events: BTreeMap<u64, Vec<EventBody>>,
    precision: i32,
    root: Handle,
}

impl Design {
    pub fn new(top: &str) -> Design {
        let mut d =
            Design { nodes: Vec::new(), procs: Vec::new(), events: BTreeMap::new(), precision: -12, root: Handle(0) };
        d.root = d.add_node(None, top, ObjKind::Module, 0);
        d
    }

    /// Time precision exponent (default `-12`, picoseconds).
    pub fn precision(mut self, exp: i32) -> Design {
        self.precision = exp;
        self
    }

    pub fn root(&self) -> Handle {
        self.root
    }

    fn add_node(&mut self, parent: Option<Handle>, name: &str, kind: ObjKind, width: u32) -> Handle {
        let path = match parent {
            Some(p) => format!("{}.{}", self.nodes[p.0 as usize].info.path, name),
            None => name.to_string(),
        };
        let h = Handle(self.nodes.len() as u32);
        let init = if kind == ObjKind::Integer { LogicVec::zeros(width) } else { LogicVec::xs(width) };
        self.nodes.push(Node {
            info: ObjInfo {
                kind,
                name: name.to_string(),
                path,
                width,
                is_const: false,
                signed: kind == ObjKind::Integer,
                range: if width > 0 { Some((width as i64 - 1, 0)) } else { None },
                type_name: format!("{kind:?}").to_lowercase(),
            },
            children: Vec::new(),
            by_name: HashMap::new(),
            value: init.clone(),
            prev: init,
            forced: None,
            real: 0.0,
            string: String::new(),
        });
        if let Some(p) = parent {
            self.nodes[p.0 as usize].children.push(h);
            self.nodes[p.0 as usize].by_name.insert(name.to_string(), h);
        }
        h
    }

    /// Add a submodule under `parent` (use [`Design::root`] for the top).
    pub fn module(&mut self, parent: Handle, name: &str) -> Handle {
        self.add_node(Some(parent), name, ObjKind::Module, 0)
    }

    /// Add a logic signal of `width` bits to the top level.
    pub fn logic(&mut self, name: &str, width: u32) -> Handle {
        self.logic_in(self.root, name, width)
    }

    pub fn logic_in(&mut self, parent: Handle, name: &str, width: u32) -> Handle {
        let kind = if width == 1 { ObjKind::Logic } else { ObjKind::LogicVec };
        self.add_node(Some(parent), name, kind, width)
    }

    pub fn integer(&mut self, name: &str) -> Handle {
        self.add_node(Some(self.root), name, ObjKind::Integer, 32)
    }

    pub fn real(&mut self, name: &str) -> Handle {
        self.add_node(Some(self.root), name, ObjKind::Real, 64)
    }

    /// A constant (parameter).
    pub fn param(&mut self, name: &str, width: u32, value: u64) -> Handle {
        let h = self.add_node(Some(self.root), name, ObjKind::LogicVec, width);
        let n = &mut self.nodes[h.0 as usize];
        n.info.is_const = true;
        n.value = LogicVec::from_u64(width, value);
        n.prev = n.value.clone();
        h
    }

    /// Set an initial value.
    pub fn init(&mut self, h: Handle, v: impl Into<LogicVec>) {
        let n = &mut self.nodes[h.0 as usize];
        let mut v: LogicVec = v.into();
        v.resize(n.info.width);
        n.value = v.clone();
        n.prev = v;
    }

    /// A process sensitive to `sens`, run whenever any of them changes.
    pub fn process(&mut self, sens: &[Handle], body: impl FnMut(&mut ProcCtx<'_>) + 'static) {
        self.procs.push(Proc { sens: sens.to_vec(), body: Box::new(body) });
    }

    /// Run `body` once at absolute time `at` (precision steps).
    pub fn at(&mut self, at: u64, body: impl FnOnce(&mut ProcCtx<'_>) + 'static) {
        self.events.entry(at).or_default().push(Box::new(body));
    }

    /// An HDL-side clock: toggles `sig` every `half_period` steps starting
    /// at time `start` with a rising edge.
    pub fn clock(&mut self, sig: Handle, half_period: u64, start: u64) {
        fn toggle(sig: Handle, half: u64, s: &mut ProcCtx<'_>, high: bool) {
            s.set(sig, LogicVec::from_u64(1, high as u64));
            s.at_relative(half, move |s| toggle(sig, half, s, !high));
        }
        self.at(start, move |s| toggle(sig, half_period, s, true));
    }

    /// Build the backend and install the runtime.
    pub fn into_backend(self) -> MockBackend {
        MockBackend { k: Rc::new(RefCell::new(Kernel::new(self))) }
    }
}

/// Context passed to processes and scheduled events.
pub struct ProcCtx<'a> {
    k: &'a mut Kernel,
    changed: &'a HashSet<Handle>,
}

impl ProcCtx<'_> {
    pub fn get(&self, h: Handle) -> LogicVec {
        self.k.nodes[h.0 as usize].value.clone()
    }

    pub fn get_u64(&self, h: Handle) -> u64 {
        self.get(h).to_u64_lossy()
    }

    /// `true` if `h` changed in this evaluation and is now 1.
    pub fn rose(&self, h: Handle) -> bool {
        let n = &self.k.nodes[h.0 as usize];
        self.changed.contains(&h) && n.value.width() > 0 && n.value.bit(0) == Logic::One && n.prev.bit(0) != Logic::One
    }

    pub fn fell(&self, h: Handle) -> bool {
        let n = &self.k.nodes[h.0 as usize];
        self.changed.contains(&h)
            && n.value.width() > 0
            && n.value.bit(0) == Logic::Zero
            && n.prev.bit(0) != Logic::Zero
    }

    /// Blocking assignment: visible immediately.
    pub fn set(&mut self, h: Handle, v: impl Into<LogicVec>) {
        let mut v: LogicVec = v.into();
        v.resize(self.k.nodes[h.0 as usize].info.width);
        self.k.hdl_write(h, v);
    }

    /// Nonblocking assignment: applied after all processes in this delta.
    pub fn nba(&mut self, h: Handle, v: impl Into<LogicVec>) {
        let mut v: LogicVec = v.into();
        v.resize(self.k.nodes[h.0 as usize].info.width);
        self.k.nba.push((h, v));
    }

    /// Schedule `body` `delay` steps from now.
    pub fn at_relative(&mut self, delay: u64, body: impl FnOnce(&mut ProcCtx<'_>) + 'static) {
        let t = self.k.now + delay;
        self.k.events.entry(t).or_default().push(Box::new(body));
    }

    pub fn now(&self) -> u64 {
        self.k.now
    }
}

struct Kernel {
    nodes: Vec<Node>,
    procs: Vec<Proc>,
    events: BTreeMap<u64, Vec<EventBody>>,
    precision: i32,
    root: Handle,
    now: u64,
    /// Signals whose value changed since callbacks were last fired.
    dirty: HashSet<Handle>,
    /// Signals changed in the current delta, for process sensitivity.
    delta_changed: HashSet<Handle>,
    nba: Vec<(Handle, LogicVec)>,
    inertial: Vec<(Handle, LogicVec)>,
    // callbacks
    next_cb: u64,
    value_cbs: HashMap<Handle, Vec<CbId>>,
    timed: BTreeMap<u64, Vec<CbId>>,
    timed_index: HashMap<CbId, u64>,
    rw: Vec<CbId>,
    ro: Vec<CbId>,
    nts: Vec<CbId>,
    finished: bool,
    pub stats: Stats,
}

/// Counters for benchmarking the harness.
#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub reads: u64,
    pub writes: u64,
    pub callbacks: u64,
    pub timesteps: u64,
}

impl Kernel {
    fn new(d: Design) -> Kernel {
        Kernel {
            nodes: d.nodes,
            procs: d.procs,
            events: d.events,
            precision: d.precision,
            root: d.root,
            now: 0,
            dirty: HashSet::new(),
            delta_changed: HashSet::new(),
            nba: Vec::new(),
            inertial: Vec::new(),
            next_cb: 1,
            value_cbs: HashMap::new(),
            timed: BTreeMap::new(),
            timed_index: HashMap::new(),
            rw: Vec::new(),
            ro: Vec::new(),
            nts: Vec::new(),
            finished: false,
            stats: Stats::default(),
        }
    }

    fn commit(&mut self, h: Handle, v: LogicVec) -> bool {
        let n = &mut self.nodes[h.0 as usize];
        if n.value == v {
            return false;
        }
        n.prev = std::mem::replace(&mut n.value, v);
        self.dirty.insert(h);
        self.delta_changed.insert(h);
        true
    }

    /// A write from HDL (process): ignored while forced.
    fn hdl_write(&mut self, h: Handle, v: LogicVec) {
        if self.nodes[h.0 as usize].forced.is_some() {
            return;
        }
        self.commit(h, v);
    }

    /// Run processes sensitive to the current delta's changes, then apply
    /// NBAs. Returns true if anything changed.
    fn delta(&mut self) -> bool {
        let changed = std::mem::take(&mut self.delta_changed);
        if changed.is_empty() {
            return false;
        }
        let mut procs = std::mem::take(&mut self.procs);
        for p in procs.iter_mut() {
            if p.sens.iter().any(|s| changed.contains(s)) {
                let mut ctx = ProcCtx { k: self, changed: &changed };
                (p.body)(&mut ctx);
            }
        }
        self.procs = procs;
        // Restore prev for the next delta: prev tracks the value before the
        // last change, which is what edge detection needs.
        let nba = std::mem::take(&mut self.nba);
        for (h, v) in nba {
            self.hdl_write(h, v);
        }
        true
    }

    fn apply_inertial(&mut self) {
        let puts = std::mem::take(&mut self.inertial);
        for (h, v) in puts {
            self.commit(h, v);
        }
    }

    fn run_events_at(&mut self, t: u64) {
        if let Some(evs) = self.events.remove(&t) {
            let empty = HashSet::new();
            for ev in evs {
                let mut ctx = ProcCtx { k: self, changed: &empty };
                ev(&mut ctx);
            }
        }
    }

    fn take_dirty_callbacks(&mut self) -> Vec<Handle> {
        let dirty = std::mem::take(&mut self.dirty);
        let mut out: Vec<Handle> =
            dirty.into_iter().filter(|h| self.value_cbs.get(h).map(|v| !v.is_empty()).unwrap_or(false)).collect();
        out.sort_by_key(|h| h.0);
        out
    }

    fn next_time(&self) -> Option<u64> {
        let a = self.timed.keys().next().copied();
        let b = self.events.keys().next().copied();
        match (a, b) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, None) => x,
            (None, y) => y,
        }
    }
}

/// The mock backend. Clone-cheap handle to the kernel; the runtime owns one
/// copy and [`MockSim`] the other.
#[derive(Clone)]
pub struct MockBackend {
    k: Rc<RefCell<Kernel>>,
}

impl MockBackend {
    /// Install the runtime with this backend and return the simulator
    /// driver.
    pub fn install(self) -> MockSim {
        let sim = MockSim { k: self.k.clone() };
        runtime::init(Box::new(self));
        sim
    }
}

impl Backend for MockBackend {
    fn name(&self) -> &str {
        "mock"
    }
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
    fn caps(&self) -> Capabilities {
        Capabilities {
            trusts_inertial_writes: std::env::var("RIVET_TRUST_INERTIAL_WRITES").map(|v| v == "1").unwrap_or(false),
            remove_fired_callbacks: false,
            four_state: true,
            supports_force: true,
        }
    }
    fn precision(&self) -> i32 {
        self.k.borrow().precision
    }
    fn now(&self) -> u64 {
        self.k.borrow().now
    }
    fn root(&mut self, name: Option<&str>) -> Result<Handle> {
        let k = self.k.borrow();
        let r = k.root;
        match name {
            Some(n) if n != k.nodes[r.0 as usize].info.name => Err(BackendError::NotFound(n.to_string())),
            _ => Ok(r),
        }
    }
    fn child_by_name(&mut self, parent: Handle, name: &str) -> Result<Option<Handle>> {
        Ok(self.k.borrow().nodes[parent.0 as usize].by_name.get(name).copied())
    }
    fn child_by_index(&mut self, parent: Handle, index: i64) -> Result<Option<Handle>> {
        Ok(self.k.borrow().nodes[parent.0 as usize].children.get(index as usize).copied())
    }
    fn children(&mut self, parent: Handle) -> Result<Vec<Handle>> {
        Ok(self.k.borrow().nodes[parent.0 as usize].children.clone())
    }
    fn info(&self, h: Handle) -> &ObjInfo {
        // SAFETY: nodes are never removed or reallocated after construction,
        // and ObjInfo is never mutated after construction.
        let k = self.k.borrow();
        let p: *const ObjInfo = &k.nodes[h.0 as usize].info;
        unsafe { &*p }
    }
    fn read(&mut self, h: Handle) -> Result<OwnedValue> {
        let mut k = self.k.borrow_mut();
        k.stats.reads += 1;
        let n = &k.nodes[h.0 as usize];
        Ok(match n.info.kind {
            ObjKind::Real => OwnedValue::Real(n.real),
            ObjKind::String => OwnedValue::Str(n.string.clone()),
            ObjKind::Integer => OwnedValue::Int(n.value.to_i64().unwrap_or(0)),
            _ => OwnedValue::Vec(n.value.clone()),
        })
    }
    fn read_vec(&mut self, h: Handle, out: &mut LogicVec) -> Result<()> {
        let mut k = self.k.borrow_mut();
        k.stats.reads += 1;
        let n = &k.nodes[h.0 as usize];
        if n.info.kind.is_hierarchy() {
            return Err(BackendError::WrongKind { path: n.info.path.clone(), expected: "value", actual: n.info.kind });
        }
        *out = n.value.clone();
        Ok(())
    }
    fn write(&mut self, h: Handle, v: Value<'_>, action: Action) -> Result<()> {
        let mut k = self.k.borrow_mut();
        k.stats.writes += 1;
        let width = k.nodes[h.0 as usize].info.width;
        if k.nodes[h.0 as usize].info.is_const {
            return Err(BackendError::Sim(format!("{} is constant", k.nodes[h.0 as usize].info.path)));
        }
        let vec = match v {
            Value::Vec(x) => {
                let mut x = x.clone();
                x.resize(width);
                x
            }
            Value::Int(i) => LogicVec::from_i64(width, i),
            Value::Real(r) => {
                k.nodes[h.0 as usize].real = r;
                return Ok(());
            }
            Value::Str(s) => {
                k.nodes[h.0 as usize].string = s.to_string();
                return Ok(());
            }
        };
        match action {
            Action::Deposit => k.inertial.push((h, vec)),
            Action::NoDelay => {
                if k.nodes[h.0 as usize].forced.is_none() {
                    k.commit(h, vec);
                }
            }
            Action::Force => {
                k.nodes[h.0 as usize].forced = Some(vec.clone());
                k.commit(h, vec);
            }
            Action::Release => {
                k.nodes[h.0 as usize].forced = None;
            }
        }
        Ok(())
    }
    fn register(&mut self, kind: CbKind) -> Result<CbId> {
        let mut k = self.k.borrow_mut();
        let id = CbId(k.next_cb);
        k.next_cb += 1;
        match kind {
            CbKind::ValueChange(h) => k.value_cbs.entry(h).or_default().push(id),
            CbKind::AfterDelay(steps) => {
                let t = k.now + steps;
                k.timed.entry(t).or_default().push(id);
                k.timed_index.insert(id, t);
            }
            CbKind::ReadWrite => k.rw.push(id),
            CbKind::ReadOnly => k.ro.push(id),
            CbKind::NextTimeStep => k.nts.push(id),
        }
        Ok(id)
    }
    fn remove(&mut self, id: CbId) -> Result<()> {
        let mut k = self.k.borrow_mut();
        if let Some(t) = k.timed_index.remove(&id) {
            if let Some(v) = k.timed.get_mut(&t) {
                v.retain(|x| *x != id);
                if v.is_empty() {
                    k.timed.remove(&t);
                }
            }
            return Ok(());
        }
        for v in k.value_cbs.values_mut() {
            v.retain(|x| *x != id);
        }
        k.rw.retain(|x| *x != id);
        k.ro.retain(|x| *x != id);
        k.nts.retain(|x| *x != id);
        Ok(())
    }
    fn finish(&mut self) {
        self.k.borrow_mut().finished = true;
    }
}

/// Drives the kernel. Obtained from [`MockBackend::install`].
pub struct MockSim {
    k: Rc<RefCell<Kernel>>,
}

impl MockSim {
    /// Run until the simulation finishes or runs out of events.
    pub fn run(&mut self) {
        runtime::dispatch(Event::StartOfSim);
        self.k.borrow_mut().stats.callbacks += 1;
        // Initial values are "changes" for sensitivity purposes.
        {
            let mut k = self.k.borrow_mut();
            let all: Vec<Handle> = (0..k.nodes.len() as u32).map(Handle).collect();
            for h in all {
                if !k.nodes[h.0 as usize].info.kind.is_hierarchy() {
                    k.delta_changed.insert(h);
                }
            }
            k.dirty.clear();
        }
        self.k.borrow_mut().run_events_at(0);
        loop {
            self.timestep();
            if self.k.borrow().finished {
                break;
            }
            let next = self.k.borrow().next_time();
            let Some(next) = next else { break };
            {
                let mut k = self.k.borrow_mut();
                k.now = next;
                k.stats.timesteps += 1;
            }
            let nts = std::mem::take(&mut self.k.borrow_mut().nts);
            if !nts.is_empty() {
                self.fire(Event::NextTimeStep);
            }
            let due = self.k.borrow_mut().timed.remove(&next).unwrap_or_default();
            for id in due {
                self.k.borrow_mut().timed_index.remove(&id);
                self.fire(Event::Timer(id));
            }
            self.k.borrow_mut().run_events_at(next);
        }
        runtime::dispatch(Event::EndOfSim);
    }

    fn fire(&mut self, ev: Event) {
        self.k.borrow_mut().stats.callbacks += 1;
        runtime::dispatch(ev);
    }

    /// Evaluate the current time step to completion.
    fn timestep(&mut self) {
        loop {
            // Evaluation cycles: apply inertial puts, tell the harness about
            // every changed signal *before* the HDL reacts to it (cocotb's
            // values-change phase), then run sensitive processes and NBAs.
            loop {
                self.k.borrow_mut().apply_inertial();
                loop {
                    let cbs = self.k.borrow_mut().take_dirty_callbacks();
                    if cbs.is_empty() {
                        break;
                    }
                    for h in cbs {
                        self.fire(Event::ValueChange(h));
                    }
                }
                if self.k.borrow().finished {
                    return;
                }
                let changed = self.k.borrow_mut().delta();
                let more = {
                    let k = self.k.borrow();
                    !k.delta_changed.is_empty() || !k.inertial.is_empty() || !k.dirty.is_empty()
                };
                if !changed && !more {
                    break;
                }
            }
            let rw = std::mem::take(&mut self.k.borrow_mut().rw);
            if rw.is_empty() {
                break;
            }
            self.fire(Event::ReadWrite);
            let more = {
                let k = self.k.borrow();
                !k.delta_changed.is_empty() || !k.inertial.is_empty() || !k.dirty.is_empty()
            };
            if !more {
                break;
            }
        }
        let ro = std::mem::take(&mut self.k.borrow_mut().ro);
        if !ro.is_empty() {
            self.fire(Event::ReadOnly);
        }
    }

    pub fn stats(&self) -> Stats {
        self.k.borrow().stats.clone()
    }

    pub fn now(&self) -> u64 {
        self.k.borrow().now
    }

    /// Peek at a signal value from outside the runtime (after the run).
    pub fn value(&self, h: Handle) -> LogicVec {
        self.k.borrow().nodes[h.0 as usize].value.clone()
    }
}

/// Run one async test body against a design, with the runtime installed
/// for the duration. Returns the test's result, or an error if the
/// simulation ran out of events before the body finished.
pub fn run_test<F, Fut>(design: Design, body: F) -> rivet_core::Result<()>
where
    F: FnOnce(rivet_core::Module) -> Fut + 'static,
    Fut: std::future::Future<Output = rivet_core::Result<()>> + 'static,
{
    rivet_core::log::init();
    let mut sim = design.into_backend().install();
    let result: Rc<RefCell<Option<rivet_core::Result<()>>>> = Rc::new(RefCell::new(None));
    let slot = result.clone();
    runtime::set_entry(move || {
        let root = runtime::backend(|b| b.root(None)).expect("root");
        runtime::set_root(root);
        runtime::begin_test();
        rivet_core::spawn_named("test", async move {
            let r = body(rivet_core::Module::from_handle(root)).await;
            *slot.borrow_mut() = Some(r);
            runtime::finish();
        });
        rivet_core::spawn_named("failure-watch", async move {
            let _ = runtime::test_failed().await;
            runtime::finish();
        });
    });
    sim.run();
    let out = result.borrow_mut().take();
    let failure = runtime::end_test();
    runtime::shutdown();
    match (out, failure) {
        (_, Some(f)) => Err(rivet_core::Error::Msg(f)),
        (Some(r), None) => r,
        (None, None) => Err(rivet_core::Error::Msg("simulation ended before the test body completed".into())),
    }
}
