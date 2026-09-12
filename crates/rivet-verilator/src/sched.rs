//! Native scheduling for Verilator.
//!
//! Because Rivet owns the simulation loop, timers and phase callbacks do
//! not need to go through Verilator's VPI callback lists (which cost an
//! allocation, a map insert, and a removal per registration). This backend
//! keeps them in a Rust timer wheel and delegates hierarchy, values, and
//! value-change callbacks to [`rivet_vpi::VpiBackend`].

use rivet_core::backend::*;
use rivet_core::value::LogicVec;
use rivet_vpi::VpiBackend;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

const NATIVE_BIT: u64 = 1 << 63;

#[derive(Default)]
pub struct NativeSched {
    timers: BTreeMap<u64, Vec<CbId>>,
    timer_index: HashMap<u64, u64>,
    pub rw: bool,
    pub ro: bool,
    pub nts: bool,
    next: u64,
    pub finished: bool,
}

impl NativeSched {
    pub fn next_deadline(&self) -> Option<u64> {
        self.timers.keys().next().copied()
    }

    /// Timers due at or before `now`, in registration order.
    pub fn take_due(&mut self, now: u64) -> Vec<CbId> {
        let mut out = Vec::new();
        while let Some((&t, _)) = self.timers.iter().next() {
            if t > now {
                break;
            }
            let ids = self.timers.remove(&t).unwrap();
            for id in &ids {
                self.timer_index.remove(&id.0);
            }
            out.extend(ids);
        }
        out
    }
}

pub struct VerilatorBackend {
    vpi: VpiBackend,
    sched: Rc<RefCell<NativeSched>>,
}

impl VerilatorBackend {
    pub fn new(vpi: VpiBackend) -> (VerilatorBackend, Rc<RefCell<NativeSched>>) {
        let sched = Rc::new(RefCell::new(NativeSched { next: 1, ..Default::default() }));
        (VerilatorBackend { vpi, sched: sched.clone() }, sched)
    }
}

impl Backend for VerilatorBackend {
    fn name(&self) -> &str {
        self.vpi.name()
    }
    fn version(&self) -> String {
        self.vpi.version()
    }
    fn caps(&self) -> Capabilities {
        self.vpi.caps()
    }
    fn precision(&self) -> i32 {
        self.vpi.precision()
    }
    fn now(&self) -> u64 {
        self.vpi.now()
    }
    fn root(&mut self, name: Option<&str>) -> Result<Handle> {
        self.vpi.root(name)
    }
    fn child_by_name(&mut self, parent: Handle, name: &str) -> Result<Option<Handle>> {
        self.vpi.child_by_name(parent, name)
    }
    fn child_by_index(&mut self, parent: Handle, index: i64) -> Result<Option<Handle>> {
        self.vpi.child_by_index(parent, index)
    }
    fn children(&mut self, parent: Handle) -> Result<Vec<Handle>> {
        self.vpi.children(parent)
    }
    fn info(&self, h: Handle) -> &ObjInfo {
        self.vpi.info(h)
    }
    fn read(&mut self, h: Handle) -> Result<OwnedValue> {
        self.vpi.read(h)
    }
    fn read_vec(&mut self, h: Handle, out: &mut LogicVec) -> Result<()> {
        self.vpi.read_vec(h, out)
    }
    fn write(&mut self, h: Handle, v: Value<'_>, action: Action) -> Result<()> {
        self.vpi.write(h, v, action)
    }
    fn register(&mut self, kind: CbKind) -> Result<CbId> {
        match kind {
            CbKind::ValueChange(_) => self.vpi.register(kind),
            _ => {
                let mut s = self.sched.borrow_mut();
                let id = CbId(NATIVE_BIT | s.next);
                s.next += 1;
                match kind {
                    CbKind::AfterDelay(steps) => {
                        let t = self.vpi.now() + steps;
                        s.timers.entry(t).or_default().push(id);
                        s.timer_index.insert(id.0, t);
                    }
                    CbKind::ReadWrite => s.rw = true,
                    CbKind::ReadOnly => s.ro = true,
                    CbKind::NextTimeStep => s.nts = true,
                    CbKind::ValueChange(_) => unreachable!(),
                }
                Ok(id)
            }
        }
    }
    fn remove(&mut self, id: CbId) -> Result<()> {
        if id.0 & NATIVE_BIT == 0 {
            return self.vpi.remove(id);
        }
        let mut s = self.sched.borrow_mut();
        if let Some(t) = s.timer_index.remove(&id.0) {
            if let Some(v) = s.timers.get_mut(&t) {
                v.retain(|x| *x != id);
                if v.is_empty() {
                    s.timers.remove(&t);
                }
            }
        }
        Ok(())
    }
    fn finish(&mut self) {
        self.sched.borrow_mut().finished = true;
        self.vpi.finish();
    }
    fn argv(&self) -> Vec<String> {
        self.vpi.argv()
    }
}
