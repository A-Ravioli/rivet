//! Native scheduling for Verilator.
//!
//! Because Rivet owns the simulation loop, timers and phase callbacks do
//! not need to go through Verilator's VPI callback lists (which cost an
//! allocation, a map insert, and a removal per registration). This backend
//! keeps them in a Rust timer wheel and delegates hierarchy, values, and
//! value-change callbacks to [`rivet_vpi::VpiBackend`].

use rivet_core::backend::*;
use rivet_core::fxhash::FxHashMap as HashMap;
use rivet_core::value::LogicVec;
use rivet_vpi::VpiBackend;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::rc::Rc;

extern "C" {
    fn rivet_vl_var_find(
        scope: *const c_char,
        name: *const c_char,
        datap: *mut *mut c_void,
        vltype: *mut c_int,
        width: *mut c_int,
        is_param: *mut c_int,
    ) -> c_int;
}

// VerilatedVarType
const VLVT_UINT8: c_int = 2;
const VLVT_UINT16: c_int = 3;
const VLVT_UINT32: c_int = 4;
const VLVT_UINT64: c_int = 5;
const VLVT_WDATA: c_int = 6;
const VLVT_REAL: c_int = 8;

/// Storage of a public variable inside the Verilated model.
#[derive(Copy, Clone)]
struct Direct {
    ptr: *mut c_void,
    vltype: c_int,
    width: u32,
    is_param: bool,
}

impl Direct {
    fn read_into(&self, out: &mut LogicVec) {
        if out.width() != self.width {
            out.resize(self.width);
        }
        let (aval, bval) = out.planes_mut();
        for b in bval.iter_mut() {
            *b = 0;
        }
        // SAFETY: ptr points at storage of the declared type, owned by the
        // model for the life of the simulation.
        unsafe {
            match self.vltype {
                VLVT_UINT8 => aval[0] = *(self.ptr as *const u8) as u32,
                VLVT_UINT16 => aval[0] = *(self.ptr as *const u16) as u32,
                VLVT_UINT32 => aval[0] = *(self.ptr as *const u32),
                VLVT_UINT64 => {
                    let v = *(self.ptr as *const u64);
                    aval[0] = v as u32;
                    if aval.len() > 1 {
                        aval[1] = (v >> 32) as u32;
                    }
                }
                VLVT_WDATA => {
                    let words = self.ptr as *const u32;
                    for (i, a) in aval.iter_mut().enumerate() {
                        *a = *words.add(i);
                    }
                }
                _ => unreachable!(),
            }
        }
        out.mask_top();
    }

    fn write(&self, v: &LogicVec) {
        // Verilator is two-state: X/Z bits become 0 (LogicVec::to_u64_lossy).
        let mask = |w: u32, bits: u32| if bits >= 32 { w } else { w & ((1u32 << bits) - 1) };
        unsafe {
            match self.vltype {
                VLVT_UINT8 | VLVT_UINT16 | VLVT_UINT32 | VLVT_UINT64 => {
                    let mut q = v.to_u64_lossy();
                    if self.width < 64 {
                        q &= (1u64 << self.width) - 1;
                    }
                    match self.vltype {
                        VLVT_UINT8 => *(self.ptr as *mut u8) = q as u8,
                        VLVT_UINT16 => *(self.ptr as *mut u16) = q as u16,
                        VLVT_UINT32 => *(self.ptr as *mut u32) = q as u32,
                        _ => *(self.ptr as *mut u64) = q,
                    }
                }
                VLVT_WDATA => {
                    let words = self.ptr as *mut u32;
                    let n = (self.width as usize).div_ceil(32);
                    for i in 0..n {
                        let w = v.aval().get(i).copied().unwrap_or(0) & !v.bval().get(i).copied().unwrap_or(0);
                        let bits = (self.width - 32 * i as u32).min(32);
                        *words.add(i) = mask(w, bits);
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}

const NATIVE_BIT: u64 = 1 << 63;

#[derive(Default)]
pub struct NativeSched {
    /// Keyed by (deadline, sequence) so same-time timers fire in
    /// registration order without a per-deadline allocation.
    timers: BTreeMap<(u64, u64), CbId>,
    timer_index: HashMap<u64, (u64, u64)>,
    pub rw: bool,
    pub ro: bool,
    pub nts: bool,
    next: u64,
    pub finished: bool,
}

impl NativeSched {
    pub fn next_deadline(&self) -> Option<u64> {
        self.timers.keys().next().map(|k| k.0)
    }

    /// Timers due at or before `now`, in registration order.
    pub fn take_due(&mut self, now: u64) -> Vec<CbId> {
        let mut out = Vec::new();
        while let Some((&key, &id)) = self.timers.iter().next() {
            if key.0 > now {
                break;
            }
            self.timers.remove(&key);
            self.timer_index.remove(&id.0);
            out.push(id);
        }
        out
    }
}

pub struct VerilatorBackend {
    vpi: VpiBackend,
    sched: Rc<RefCell<NativeSched>>,
    /// Direct storage per handle, `None` where VPI must be used.
    direct: HashMap<Handle, Option<Direct>>,
    use_direct: bool,
}

impl VerilatorBackend {
    pub fn new(vpi: VpiBackend) -> (VerilatorBackend, Rc<RefCell<NativeSched>>) {
        let sched = Rc::new(RefCell::new(NativeSched { next: 1, ..Default::default() }));
        let use_direct = std::env::var("RIVET_VERILATOR_DIRECT").map(|v| v != "0").unwrap_or(true);
        (VerilatorBackend { vpi, sched: sched.clone(), direct: HashMap::default(), use_direct }, sched)
    }

    /// Resolve (and cache) the model storage behind a handle.
    fn direct(&mut self, h: Handle) -> Option<Direct> {
        if !self.use_direct {
            return None;
        }
        if let Some(d) = self.direct.get(&h) {
            return *d;
        }
        let info = self.vpi.info(h);
        let found = if info.kind.is_logic_like() || info.kind == ObjKind::Real {
            let (scope, name) = match info.path.rsplit_once('.') {
                Some((s, n)) => (s.to_string(), n.to_string()),
                None => (String::new(), info.path.clone()),
            };
            let cscope = CString::new(scope).ok();
            let cname = CString::new(name).ok();
            match (cscope, cname) {
                (Some(cs), Some(cn)) => {
                    let mut datap: *mut c_void = std::ptr::null_mut();
                    let (mut vltype, mut width, mut is_param) = (0 as c_int, 0 as c_int, 0 as c_int);
                    let ok = unsafe {
                        rivet_vl_var_find(cs.as_ptr(), cn.as_ptr(), &mut datap, &mut vltype, &mut width, &mut is_param)
                    };
                    if ok == 1
                        && !datap.is_null()
                        && matches!(
                            vltype,
                            VLVT_UINT8 | VLVT_UINT16 | VLVT_UINT32 | VLVT_UINT64 | VLVT_WDATA | VLVT_REAL
                        )
                        && (vltype == VLVT_REAL || width as u32 == info.width)
                    {
                        Some(Direct { ptr: datap, vltype, width: width as u32, is_param: is_param != 0 })
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        self.direct.insert(h, found);
        found
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
        if let Some(d) = self.direct(h) {
            if d.vltype == VLVT_REAL {
                return Ok(OwnedValue::Real(unsafe { *(d.ptr as *const f64) }));
            }
            let mut v = LogicVec::zeros(d.width);
            d.read_into(&mut v);
            return Ok(if self.vpi.info(h).kind == ObjKind::Integer {
                OwnedValue::Int(v.to_i64().unwrap_or(0))
            } else {
                OwnedValue::Vec(v)
            });
        }
        self.vpi.read(h)
    }
    fn read_vec(&mut self, h: Handle, out: &mut LogicVec) -> Result<()> {
        if let Some(d) = self.direct(h) {
            if d.vltype != VLVT_REAL {
                d.read_into(out);
                return Ok(());
            }
        }
        self.vpi.read_vec(h, out)
    }
    fn write(&mut self, h: Handle, v: Value<'_>, action: Action) -> Result<()> {
        if matches!(action, Action::Deposit | Action::NoDelay) {
            if let Some(d) = self.direct(h) {
                if d.is_param {
                    return Err(BackendError::Sim(format!("{} is a parameter", self.vpi.info(h).path)));
                }
                match (&v, d.vltype) {
                    (Value::Vec(x), t) if t != VLVT_REAL => {
                        d.write(x);
                        return Ok(());
                    }
                    (Value::Int(i), t) if t != VLVT_REAL => {
                        d.write(&LogicVec::from_i64(d.width, *i));
                        return Ok(());
                    }
                    (Value::Real(r), VLVT_REAL) => {
                        unsafe { *(d.ptr as *mut f64) = *r };
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
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
                        let key = (self.vpi.now() + steps, id.0);
                        s.timers.insert(key, id);
                        s.timer_index.insert(id.0, key);
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
        if let Some(key) = s.timer_index.remove(&id.0) {
            s.timers.remove(&key);
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
