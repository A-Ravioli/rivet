//! VPI backend: Icarus, Verilator (through its VPI shim), Questa/ModelSim
//! Verilog, Xcelium, VCS, Riviera, DSim.
//!
//! The user's test crate is a `cdylib` that depends on this crate; the
//! exported `vlog_startup_routines` table makes any VPI-capable simulator
//! load it directly. See `docs/design/01-architecture.md` §3.2.

#![allow(non_upper_case_globals)]

pub mod ffi;

use ffi::*;
use rivet_core::backend::*;
use rivet_core::runtime::{self, Event};
use rivet_core::value::LogicVec;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Which simulator is hosting us; selects the quirks catalogued in
/// `docs/design/00-cocotb-analysis.md` §3.6.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Sim {
    Icarus,
    Verilator,
    Questa,
    Xcelium,
    Vcs,
    Riviera,
    Dsim,
    Ghdl,
    Other,
}

struct Entry {
    raw: vpiHandle,
    info: ObjInfo,
    children: Option<HashMap<String, Handle>>,
}

struct CbRec {
    id: CbId,
    kind: CbKind,
    cb_handle: vpiHandle,
    time: s_vpi_time,
    value: s_vpi_value,
    data: s_cb_data,
    removed: bool,
}

thread_local! {
    /// Callback records live outside the backend so the C callback can find
    /// them without borrowing the runtime.
    static CALLBACKS: RefCell<rivet_core::fxhash::FxHashMap<u64, Box<CbRec>>> = RefCell::new(Default::default());
    static NEXT_CB: RefCell<u64> = const { RefCell::new(1) };
    static SIM: RefCell<Sim> = const { RefCell::new(Sim::Other) };
}

/// The VPI backend.
pub struct VpiBackend {
    sim: Sim,
    product: String,
    version: String,
    entries: Vec<Entry>,
    by_path: HashMap<String, Handle>,
    /// Resolved lazily: not available before elaboration.
    precision: std::cell::Cell<Option<i32>>,
    /// `vpi_put_value` flag used for `Action::Deposit`.
    deposit_flag: i32,
    trusts_inertial: bool,
    vec_buf: Vec<s_vpi_vecval>,
    /// GHDL's VPI has no vpiVectorVal; move values as binary strings.
    string_values: bool,
    str_buf: Vec<u8>,
}

impl VpiBackend {
    /// Query the simulator and build a backend with the right quirks.
    pub fn new() -> VpiBackend {
        let (product, version) = unsafe {
            let mut info = s_vpi_vlog_info {
                argc: 0,
                argv: std::ptr::null_mut(),
                product: std::ptr::null_mut(),
                version: std::ptr::null_mut(),
            };
            let ok = vpi_get_vlog_info(&mut info);
            if ok != 0 {
                (cstr(info.product), cstr(info.version))
            } else {
                (String::new(), String::new())
            }
        };
        let pl = product.to_ascii_lowercase();
        let sim = if pl.contains("icarus") {
            Sim::Icarus
        } else if pl.contains("verilator") {
            Sim::Verilator
        } else if pl.contains("modelsim") || pl.contains("questa") {
            Sim::Questa
        } else if pl.contains("xcelium") || pl.contains("ncsim") || pl.contains("xmsim") {
            Sim::Xcelium
        } else if pl.contains("vcs") {
            Sim::Vcs
        } else if pl.contains("riviera") || pl.contains("aldec") {
            Sim::Riviera
        } else if pl.contains("dsim") {
            Sim::Dsim
        } else if pl.contains("ghdl") {
            Sim::Ghdl
        } else {
            Sim::Other
        };
        SIM.with(|s| *s.borrow_mut() = sim);
        // Verilator gained vpiInertialDelay in 5.036, but honouring it needs
        // the simulation loop to call VerilatedVpi::doInertialPuts. Rivet
        // owns that loop and instead buffers deposits in the runtime until
        // the ReadWrite phase and applies them as immediate writes, which
        // behaves identically on every Verilator version and matches the
        // direct-access path. So Verilator never trusts inertial writes.
        let deposit_flag = if sim == Sim::Verilator { vpiNoDelay } else { vpiInertialDelay };
        let trusts_inertial = match std::env::var("RIVET_TRUST_INERTIAL_WRITES") {
            Ok(v) => v == "1" && sim != Sim::Verilator,
            // cocotb's defaults: trust GHDL, not Icarus/Questa/Xcelium/VCS.
            Err(_) => sim == Sim::Ghdl,
        };
        VpiBackend {
            sim,
            product,
            version,
            entries: Vec::new(),
            by_path: HashMap::new(),
            precision: std::cell::Cell::new(None),
            deposit_flag,
            trusts_inertial,
            vec_buf: Vec::new(),
            string_values: sim == Sim::Ghdl,
            str_buf: Vec::new(),
        }
    }

    pub fn sim(&self) -> Sim {
        self.sim
    }

    fn intern(&mut self, raw: vpiHandle, parent_path: Option<&str>, fallback_name: &str) -> Handle {
        let vtype = unsafe { vpi_get(vpiType, raw) };
        let name = unsafe { cstr(vpi_get_str(vpiName, raw)) };
        let name = if name.is_empty() { fallback_name.to_string() } else { name };
        let full = unsafe { cstr(vpi_get_str(vpiFullName, raw)) };
        let path = if !full.is_empty() {
            full
        } else {
            match parent_path {
                Some(p) => format!("{p}.{name}"),
                None => name.clone(),
            }
        };
        if let Some(&h) = self.by_path.get(&path) {
            // Duplicate discovery of the same object: keep one handle, as
            // cocotb's GpiHandleStore does.
            unsafe { vpi_free_object(raw) };
            return h;
        }
        let info = self.classify(raw, vtype, name, path.clone());
        let h = Handle(self.entries.len() as u32);
        self.entries.push(Entry { raw, info, children: None });
        self.by_path.insert(path, h);
        h
    }

    fn classify(&self, raw: vpiHandle, vtype: i32, name: String, path: String) -> ObjInfo {
        let size = unsafe { vpi_get(vpiSize, raw) };
        let (kind, width, is_const) = match vtype {
            vpiModule | vpiInterface | vpiProgram | vpiGenScope | vpiInternalScope | vpiPort | vpiScope | vpiBegin
            | vpiNamedBegin => (ObjKind::Module, 0, false),
            vpiGenScopeArray | vpiModuleArray | vpiInterfaceArray => (ObjKind::GenArray, size.max(0) as u32, false),
            vpiPackage => (ObjKind::Package, 0, false),
            vpiStructVar | vpiStructNet => {
                if self.sim != Sim::Ghdl && unsafe { vpi_get(vpiPacked, raw) } == 1 {
                    (ObjKind::LogicVec, size.max(0) as u32, false)
                } else {
                    (ObjKind::Struct, 0, false)
                }
            }
            vpiNetArray | vpiRegArray | vpiMemory => (ObjKind::Array, size.max(0) as u32, false),
            vpiRealVar => (ObjKind::Real, 64, false),
            vpiIntegerVar | vpiIntVar | vpiLongIntVar | vpiShortIntVar | vpiByteVar | vpiTimeVar => {
                (ObjKind::Integer, size.max(0) as u32, false)
            }
            vpiEnumVar => (ObjKind::Enum, size.max(0) as u32, false),
            vpiStringVar => (ObjKind::String, 0, false),
            vpiParameter | vpiConstant => {
                let ct = if self.sim == Sim::Ghdl { vpiUndefined } else { unsafe { vpi_get(vpiConstType, raw) } };
                match ct {
                    vpiRealConst => (ObjKind::Real, 64, true),
                    vpiStringConst => (ObjKind::String, 0, true),
                    _ => (ObjKind::LogicVec, size.max(0) as u32, true),
                }
            }
            vpiNet | vpiReg | vpiRegBit | vpiBitVar | vpiMemoryWord | vpiPackedArrayVar | vpiBitSelect
            | vpiPartSelect | vpiVarSelect | vpiArrayMember => {
                if size == 1 {
                    (ObjKind::Logic, 1, false)
                } else {
                    (ObjKind::LogicVec, size.max(0) as u32, false)
                }
            }
            _ => {
                log::debug!("unknown VPI object type {vtype} for {path}");
                (ObjKind::Unknown, size.max(0) as u32, false)
            }
        };
        // GHDL prints a warning for properties it does not know.
        let signed = self.sim != Sim::Ghdl && unsafe { vpi_get(vpiSigned, raw) } == 1;
        let range = if kind.is_logic_like() || kind == ObjKind::Array {
            unsafe {
                let l = vpi_handle(vpiLeftRange, raw);
                let r = vpi_handle(vpiRightRange, raw);
                if !l.is_null() && !r.is_null() {
                    let mut lv = s_vpi_value::new(vpiIntVal);
                    let mut rv = s_vpi_value::new(vpiIntVal);
                    vpi_get_value(l, &mut lv);
                    vpi_get_value(r, &mut rv);
                    let res = Some((lv.value.integer as i64, rv.value.integer as i64));
                    vpi_free_object(l);
                    vpi_free_object(r);
                    res
                } else {
                    None
                }
            }
        } else {
            None
        };
        ObjInfo { kind, name, path, width, is_const, signed, range, type_name: vpi_type_name(vtype) }
    }

    fn raw(&self, h: Handle) -> vpiHandle {
        self.entries[h.0 as usize].raw
    }

    fn check_error(&self, what: &str) -> Result<()> {
        unsafe {
            let mut info = std::mem::zeroed::<s_vpi_error_info>();
            let level = vpi_chk_error(&mut info);
            if level >= vpiError {
                return Err(BackendError::Sim(format!("{what}: {}", cstr(info.message))));
            }
        }
        Ok(())
    }

    /// Read a value as a binary string and parse it (fallback path).
    fn read_binstr(&self, raw: vpiHandle) -> Result<LogicVec> {
        let mut v = s_vpi_value::new(vpiBinStrVal);
        unsafe {
            vpi_get_value(raw, &mut v);
            let s = cstr(v.value.str_);
            LogicVec::from_binstr(&s).ok_or_else(|| BackendError::Sim(format!("unparseable binstr {s:?}")))
        }
    }
}

impl Default for VpiBackend {
    fn default() -> Self {
        VpiBackend::new()
    }
}

fn vpi_type_name(t: i32) -> String {
    match t {
        vpiModule => "module",
        vpiNet => "net",
        vpiReg => "reg",
        vpiIntegerVar => "integer",
        vpiRealVar => "real",
        vpiParameter => "parameter",
        vpiMemory => "memory",
        vpiRegArray => "reg array",
        vpiNetArray => "net array",
        vpiGenScope => "generate scope",
        vpiGenScopeArray => "generate array",
        vpiStringVar => "string",
        vpiIntVar => "int",
        vpiLongIntVar => "longint",
        vpiBitVar => "bit",
        vpiEnumVar => "enum",
        vpiStructVar => "struct",
        vpiPackedArrayVar => "packed array",
        vpiInterface => "interface",
        _ => return format!("vpi type {t}"),
    }
    .to_string()
}

/// Relationship types tried, in order, when enumerating a scope's children.
/// Mirrors cocotb's `VpiIterator` tables minus the entries it disables.
const MODULE_CHILDREN: &[i32] = &[
    vpiNet,
    vpiNetArray,
    vpiReg,
    vpiRegArray,
    vpiMemory,
    vpiIntegerVar,
    vpiRealVar,
    vpiVariables,
    vpiParameter,
    vpiNamedEvent,
    vpiInternalScope,
    vpiModule,
];

impl Backend for VpiBackend {
    fn name(&self) -> &str {
        match self.sim {
            Sim::Icarus => "icarus",
            Sim::Verilator => "verilator",
            Sim::Questa => "questa",
            Sim::Xcelium => "xcelium",
            Sim::Vcs => "vcs",
            Sim::Riviera => "riviera",
            Sim::Dsim => "dsim",
            Sim::Ghdl => "ghdl",
            Sim::Other => "vpi",
        }
    }

    fn version(&self) -> String {
        format!("{} {}", self.product, self.version).trim().to_string()
    }

    fn caps(&self) -> Capabilities {
        Capabilities {
            trusts_inertial_writes: self.trusts_inertial,
            remove_fired_callbacks: self.sim == Sim::Verilator,
            four_state: self.sim != Sim::Verilator,
            supports_force: self.sim != Sim::Verilator,
        }
    }

    fn precision(&self) -> i32 {
        if let Some(p) = self.precision.get() {
            return p;
        }
        // Icarus answers 0 for a NULL object, so ask the first top-level
        // module. Neither works before elaboration, hence the laziness.
        let mut prec = unsafe { vpi_get(vpiTimePrecision, std::ptr::null_mut()) };
        unsafe {
            let it = vpi_iterate(vpiModule, std::ptr::null_mut());
            if !it.is_null() {
                let first = vpi_scan(it);
                if !first.is_null() {
                    let mp = vpi_get(vpiTimePrecision, first);
                    if mp != vpiUndefined {
                        prec = mp;
                    }
                    vpi_free_object(it);
                } else {
                    // Not elaborated yet: do not cache.
                    return prec.clamp(-15, 2);
                }
            }
        }
        let prec = prec.clamp(-15, 2);
        self.precision.set(Some(prec));
        prec
    }

    fn now(&self) -> u64 {
        unsafe {
            let mut t = s_vpi_time { type_: vpiSimTime, high: 0, low: 0, real: 0.0 };
            vpi_get_time(std::ptr::null_mut(), &mut t);
            ((t.high as u64) << 32) | t.low as u64
        }
    }

    fn root(&mut self, name: Option<&str>) -> Result<Handle> {
        let mut found: Vec<(String, vpiHandle)> = Vec::new();
        unsafe {
            let it = vpi_iterate(vpiModule, std::ptr::null_mut());
            if !it.is_null() {
                loop {
                    let h = vpi_scan(it);
                    if h.is_null() {
                        break;
                    }
                    let full = cstr(vpi_get_str(vpiFullName, h));
                    // Xcelium places virtual classes at the top scope with
                    // escaped names; skip them (cocotb VpiImpl.cpp:635).
                    if full.starts_with('\\') {
                        vpi_free_object(h);
                        continue;
                    }
                    found.push((cstr(vpi_get_str(vpiName, h)), h));
                }
            }
        }
        if found.is_empty() {
            return Err(BackendError::NotFound("no top-level module".into()));
        }
        let pick = match name {
            Some(n) => found.iter().position(|(nm, _)| nm == n || nm.eq_ignore_ascii_case(n)),
            None => Some(0),
        };
        let Some(i) = pick else {
            let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
            return Err(BackendError::NotFound(format!("top-level {name:?} (available: {names:?})")));
        };
        let (nm, raw) = found.swap_remove(i);
        for (_, other) in found {
            unsafe { vpi_free_object(other) };
        }
        Ok(self.intern(raw, None, &nm))
    }

    fn child_by_name(&mut self, parent: Handle, name: &str) -> Result<Option<Handle>> {
        if let Some(ch) = &self.entries[parent.0 as usize].children {
            if let Some(&h) = ch.get(name) {
                return Ok(Some(h));
            }
        }
        let praw = self.raw(parent);
        let ppath = self.entries[parent.0 as usize].info.path.clone();
        let cname = CString::new(name).map_err(|_| BackendError::NotFound(name.into()))?;
        let mut raw = unsafe { vpi_handle_by_name(cname.as_ptr(), praw) };
        if raw.is_null() {
            // Some simulators only resolve fully qualified names.
            let full = CString::new(format!("{ppath}.{name}")).unwrap();
            raw = unsafe { vpi_handle_by_name(full.as_ptr(), std::ptr::null_mut()) };
        }
        if raw.is_null() {
            // Generate blocks without vpiGenScopeArray support (Icarus,
            // Verilator, Questa): `gen[0]` exists but `gen` does not. Return
            // the parent itself typed as a generate array, resolved by name
            // on indexing, as cocotb does (VpiImpl.cpp:406-452).
            let probe = CString::new(format!("{ppath}.{name}[0]")).unwrap();
            let p = unsafe { vpi_handle_by_name(probe.as_ptr(), std::ptr::null_mut()) };
            if !p.is_null() {
                unsafe { vpi_free_object(p) };
                let path = format!("{ppath}.{name}");
                let h = Handle(self.entries.len() as u32);
                self.entries.push(Entry {
                    raw: praw,
                    info: ObjInfo {
                        kind: ObjKind::GenArray,
                        name: name.to_string(),
                        path: path.clone(),
                        width: 0,
                        is_const: false,
                        signed: false,
                        range: None,
                        type_name: "generate array".into(),
                    },
                    children: None,
                });
                self.by_path.insert(path, h);
                self.entries[parent.0 as usize].children.get_or_insert_with(HashMap::new).insert(name.to_string(), h);
                return Ok(Some(h));
            }
            return Ok(None);
        }
        let h = self.intern(raw, Some(&ppath), name);
        if h == parent {
            // GHDL answers a lookup of an unknown generic with the scope
            // itself; report it as missing instead.
            return Ok(None);
        }
        self.entries[parent.0 as usize].children.get_or_insert_with(HashMap::new).insert(name.to_string(), h);
        Ok(Some(h))
    }

    fn child_by_index(&mut self, parent: Handle, index: i64) -> Result<Option<Handle>> {
        let key = format!("[{index}]");
        if let Some(ch) = &self.entries[parent.0 as usize].children {
            if let Some(&h) = ch.get(&key) {
                return Ok(Some(h));
            }
        }
        let ppath = self.entries[parent.0 as usize].info.path.clone();
        let is_pseudo = self.entries[parent.0 as usize].info.kind == ObjKind::GenArray;
        let mut raw = std::ptr::null_mut();
        if !is_pseudo {
            raw = unsafe { vpi_handle_by_index(self.raw(parent), index as i32) };
        }
        if raw.is_null() {
            let full = CString::new(format!("{ppath}[{index}]")).unwrap();
            raw = unsafe { vpi_handle_by_name(full.as_ptr(), std::ptr::null_mut()) };
        }
        if raw.is_null() {
            return Ok(None);
        }
        let h = self.intern(raw, Some(&ppath), &key);
        self.entries[parent.0 as usize].children.get_or_insert_with(HashMap::new).insert(key, h);
        Ok(Some(h))
    }

    fn children(&mut self, parent: Handle) -> Result<Vec<Handle>> {
        let praw = self.raw(parent);
        let ppath = self.entries[parent.0 as usize].info.path.clone();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for &rel in MODULE_CHILDREN {
            let it = unsafe { vpi_iterate(rel, praw) };
            if it.is_null() {
                continue;
            }
            loop {
                let raw = unsafe { vpi_scan(it) };
                if raw.is_null() {
                    break;
                }
                let name = unsafe { cstr(vpi_get_str(vpiName, raw)) };
                if name.is_empty() {
                    unsafe { vpi_free_object(raw) };
                    continue;
                }
                let h = self.intern(raw, Some(&ppath), &name);
                if seen.insert(h) {
                    out.push(h);
                    self.entries[parent.0 as usize].children.get_or_insert_with(HashMap::new).insert(name, h);
                }
            }
        }
        Ok(out)
    }

    fn info(&self, h: Handle) -> &ObjInfo {
        &self.entries[h.0 as usize].info
    }

    fn read(&mut self, h: Handle) -> Result<OwnedValue> {
        let raw = self.raw(h);
        let kind = self.entries[h.0 as usize].info.kind;
        unsafe {
            match kind {
                ObjKind::Real => {
                    let mut v = s_vpi_value::new(vpiRealVal);
                    vpi_get_value(raw, &mut v);
                    Ok(OwnedValue::Real(v.value.real))
                }
                ObjKind::String => {
                    let mut v = s_vpi_value::new(vpiStringVal);
                    vpi_get_value(raw, &mut v);
                    Ok(OwnedValue::Str(cstr(v.value.str_)))
                }
                ObjKind::Integer => {
                    let mut out = LogicVec::zeros(0);
                    self.read_vec(h, &mut out)?;
                    Ok(OwnedValue::Int(out.to_i64().unwrap_or(0)))
                }
                _ => {
                    let mut out = LogicVec::zeros(0);
                    self.read_vec(h, &mut out)?;
                    Ok(OwnedValue::Vec(out))
                }
            }
        }
    }

    fn read_vec(&mut self, h: Handle, out: &mut LogicVec) -> Result<()> {
        let raw = self.raw(h);
        let info = &self.entries[h.0 as usize].info;
        if info.kind.is_hierarchy() {
            return Err(BackendError::WrongKind { path: info.path.clone(), expected: "value", actual: info.kind });
        }
        let width = info.width;
        if width == 0 {
            *out = self.read_binstr(raw)?;
            return Ok(());
        }
        if self.string_values {
            *out = self.read_binstr(raw)?;
            if out.width() != width && width > 0 {
                out.resize(width);
            }
            return Ok(());
        }
        let words = (width as usize).div_ceil(32);
        let mut v = s_vpi_value::new(vpiVectorVal);
        unsafe {
            vpi_get_value(raw, &mut v);
            let vec = v.value.vector;
            if vec.is_null() {
                *out = self.read_binstr(raw)?;
                return Ok(());
            }
            if out.width() != width {
                out.resize(width);
            }
            let (aval, bval) = out.planes_mut();
            for i in 0..words {
                let e = *vec.add(i);
                aval[i] = e.aval;
                bval[i] = e.bval;
            }
        }
        out.mask_top();
        Ok(())
    }

    fn write(&mut self, h: Handle, v: Value<'_>, action: Action) -> Result<()> {
        let raw = self.raw(h);
        let info = &self.entries[h.0 as usize].info;
        if info.is_const {
            return Err(BackendError::Sim(format!("{} is a constant", info.path)));
        }
        let mut time = s_vpi_time { type_: vpiSimTime, high: 0, low: 0, real: 0.0 };
        let flag = match action {
            Action::Deposit => {
                // Questa and Xcelium reject inertial writes to string
                // variables (cocotb VpiSignal.cpp:195-206).
                if info.kind == ObjKind::String && matches!(self.sim, Sim::Questa | Sim::Xcelium) {
                    vpiNoDelay
                } else {
                    self.deposit_flag
                }
            }
            Action::NoDelay => vpiNoDelay,
            Action::Force => vpiForceFlag,
            Action::Release => vpiReleaseFlag,
        };
        let cstring;
        let mut val = match v {
            Value::Vec(x) if self.string_values => {
                self.str_buf.clear();
                self.str_buf.extend(x.to_binstr().bytes());
                self.str_buf.push(0);
                let mut val = s_vpi_value::new(vpiBinStrVal);
                val.value.str_ = self.str_buf.as_mut_ptr() as *mut c_char;
                val
            }
            Value::Vec(x) => {
                let words = (x.width() as usize).div_ceil(32);
                self.vec_buf.clear();
                self.vec_buf.extend((0..words).map(|i| s_vpi_vecval { aval: x.aval()[i], bval: x.bval()[i] }));
                if self.vec_buf.is_empty() {
                    self.vec_buf.push(s_vpi_vecval::default());
                }
                let mut val = s_vpi_value::new(vpiVectorVal);
                val.value.vector = self.vec_buf.as_mut_ptr();
                val
            }
            Value::Int(i) => {
                let mut val = s_vpi_value::new(vpiIntVal);
                val.value.integer = i as i32;
                val
            }
            Value::Real(r) => {
                let mut val = s_vpi_value::new(vpiRealVal);
                val.value.real = r;
                val
            }
            Value::Str(s) => {
                cstring = CString::new(s).unwrap_or_default();
                let mut val = s_vpi_value::new(vpiStringVal);
                val.value.str_ = cstring.as_ptr() as *mut c_char;
                val
            }
        };
        unsafe {
            if action == Action::Release {
                // Pass the current value when releasing, as cocotb does.
                vpi_get_value(raw, &mut val);
            }
            if flag == vpiNoDelay {
                vpi_put_value(raw, &mut val, std::ptr::null_mut(), vpiNoDelay);
            } else {
                vpi_put_value(raw, &mut val, &mut time, flag);
            }
        }
        self.check_error("vpi_put_value")
    }

    fn register(&mut self, kind: CbKind) -> Result<CbId> {
        let id = NEXT_CB.with(|n| {
            let mut n = n.borrow_mut();
            let id = *n;
            *n += 1;
            CbId(id)
        });
        let (reason, obj, time, value) = match kind {
            CbKind::ValueChange(h) => (
                cbValueChange,
                self.raw(h),
                s_vpi_time { type_: vpiSuppressTime, high: 0, low: 0, real: 0.0 },
                s_vpi_value::new(vpiSuppressVal),
            ),
            CbKind::AfterDelay(steps) => (
                cbAfterDelay,
                std::ptr::null_mut(),
                s_vpi_time { type_: vpiSimTime, high: (steps >> 32) as u32, low: steps as u32, real: 0.0 },
                s_vpi_value::new(vpiSuppressVal),
            ),
            CbKind::ReadWrite => (
                cbReadWriteSynch,
                std::ptr::null_mut(),
                s_vpi_time { type_: vpiSimTime, high: 0, low: 0, real: 0.0 },
                s_vpi_value::new(vpiSuppressVal),
            ),
            CbKind::ReadOnly => (
                cbReadOnlySynch,
                std::ptr::null_mut(),
                s_vpi_time { type_: vpiSimTime, high: 0, low: 0, real: 0.0 },
                s_vpi_value::new(vpiSuppressVal),
            ),
            CbKind::NextTimeStep => (
                cbNextSimTime,
                std::ptr::null_mut(),
                s_vpi_time { type_: vpiSimTime, high: 0, low: 0, real: 0.0 },
                s_vpi_value::new(vpiSuppressVal),
            ),
        };
        register_raw(id, kind, reason, obj, time, value)
    }

    fn remove(&mut self, id: CbId) -> Result<()> {
        let rec = CALLBACKS.with(|c| c.borrow_mut().remove(&id.0));
        if let Some(mut rec) = rec {
            if !rec.removed && !rec.cb_handle.is_null() {
                let ok = unsafe { vpi_remove_cb(rec.cb_handle) };
                if ok == 0 {
                    // Could not remove; keep the record so a late fire is
                    // squashed rather than dereferencing freed memory.
                    rec.removed = true;
                    CALLBACKS.with(|c| c.borrow_mut().insert(id.0, rec));
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) {
        unsafe {
            vpi_control(vpiFinish, 0i32);
        }
    }

    fn argv(&self) -> Vec<String> {
        unsafe {
            let mut info = s_vpi_vlog_info {
                argc: 0,
                argv: std::ptr::null_mut(),
                product: std::ptr::null_mut(),
                version: std::ptr::null_mut(),
            };
            if vpi_get_vlog_info(&mut info) == 0 || info.argv.is_null() {
                return Vec::new();
            }
            (0..info.argc as usize).map(|i| cstr(*info.argv.add(i))).collect()
        }
    }
}

/// Register a callback record with the simulator.
fn register_raw(
    id: CbId,
    kind: CbKind,
    reason: i32,
    obj: vpiHandle,
    time: s_vpi_time,
    value: s_vpi_value,
) -> Result<CbId> {
    let mut rec = Box::new(CbRec {
        id,
        kind,
        cb_handle: std::ptr::null_mut(),
        time,
        value,
        data: s_cb_data {
            reason,
            cb_rtn: Some(rivet_vpi_callback),
            obj,
            time: std::ptr::null_mut(),
            value: std::ptr::null_mut(),
            index: 0,
            user_data: std::ptr::null_mut(),
        },
        removed: false,
    });
    // Point the cb_data at the boxed record's own fields; the Box gives
    // them a stable address for the life of the registration.
    let p: *mut CbRec = &mut *rec;
    unsafe {
        (*p).data.time = &mut (*p).time;
        (*p).data.value = &mut (*p).value;
        (*p).data.user_data = p as *mut c_char;
        let h = vpi_register_cb(&mut (*p).data);
        if h.is_null() {
            return Err(BackendError::Sim(format!("vpi_register_cb failed for {kind:?}")));
        }
        (*p).cb_handle = h;
    }
    CALLBACKS.with(|c| c.borrow_mut().insert(id.0, rec));
    Ok(id)
}

/// The single C entry point for every callback we register.
unsafe extern "C" fn rivet_vpi_callback(cb: *mut s_cb_data) -> i32 {
    let res = catch_unwind(AssertUnwindSafe(|| {
        let p = (*cb).user_data as *mut CbRec;
        if p.is_null() {
            return;
        }
        let (id, kind, removed) = ((*p).id, (*p).kind, (*p).removed);
        if removed {
            return;
        }
        let one_shot = !matches!(kind, CbKind::ValueChange(_));
        let sim = SIM.with(|s| *s.borrow());
        let event = match kind {
            CbKind::ValueChange(h) => Event::ValueChange(h),
            CbKind::AfterDelay(_) => Event::Timer(id),
            CbKind::ReadWrite => Event::ReadWrite,
            CbKind::ReadOnly => Event::ReadOnly,
            CbKind::NextTimeStep => Event::NextTimeStep,
        };
        // Keep the record alive across dispatch but out of the map, so a
        // `remove()` from inside user code is a no-op for a fired one-shot.
        let rec = if one_shot { CALLBACKS.with(|c| c.borrow_mut().remove(&id.0)) } else { None };
        if let Some(rec) = &rec {
            // Verilator treats one-shot callbacks as recurring; other
            // simulators free them on fire and dislike a remove afterwards
            // (cocotb VpiCbHdl.cpp:145-174).
            if sim == Sim::Verilator && !rec.cb_handle.is_null() {
                vpi_remove_cb(rec.cb_handle);
            }
        }
        runtime::dispatch(event);
        drop(rec);
    }));
    match res {
        Ok(()) => 0,
        Err(payload) => {
            let msg = runtime::panic_message(&payload);
            eprintln!("rivet: panic in VPI callback: {msg}");
            vpi_control(vpiFinish, 1i32);
            -1
        }
    }
}

unsafe extern "C" fn rivet_start_of_sim(_cb: *mut s_cb_data) -> i32 {
    let res = catch_unwind(|| runtime::dispatch(Event::StartOfSim));
    if let Err(p) = res {
        eprintln!("rivet: panic at start of simulation: {}", runtime::panic_message(&p));
        vpi_control(vpiFinish, 1i32);
        return -1;
    }
    0
}

static EXIT_CODE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Exit code decided by the regression, available after end of simulation.
pub fn exit_code() -> i32 {
    EXIT_CODE.load(std::sync::atomic::Ordering::Relaxed)
}

unsafe extern "C" fn rivet_end_of_sim(_cb: *mut s_cb_data) -> i32 {
    let _ = catch_unwind(|| {
        if !runtime::is_initialised() {
            return;
        }
        runtime::dispatch(Event::EndOfSim);
        let code = runtime::exit_code();
        EXIT_CODE.store(code, std::sync::atomic::Ordering::Relaxed);
        if code != 0 {
            eprintln!("rivet: simulation finished with failures (exit code {code})");
        }
        runtime::shutdown();
    });
    0
}

/// Initialise the runtime with a VPI backend and hook start/end of
/// simulation. Called from `vlog_startup_routines`, or directly by an
/// embedding `main` (Verilator).
pub fn startup() {
    startup_with(|b| Box::new(b));
}

/// Like [`startup`], but lets the caller wrap the [`VpiBackend`] (the
/// Verilator backend delegates hierarchy and values to it and schedules
/// timers natively).
pub fn startup_with(wrap: impl FnOnce(VpiBackend) -> Box<dyn Backend>) {
    rivet_core::log::init();
    let backend = VpiBackend::new();
    log::debug!("rivet VPI backend on {} ({:?})", backend.version(), backend.sim());
    runtime::init(wrap(backend));
    rivet_core::test::install_default_entry();
    unsafe {
        static mut START: s_cb_data = s_cb_data {
            reason: cbStartOfSimulation,
            cb_rtn: Some(rivet_start_of_sim),
            obj: std::ptr::null_mut(),
            time: std::ptr::null_mut(),
            value: std::ptr::null_mut(),
            index: 0,
            user_data: std::ptr::null_mut(),
        };
        static mut END: s_cb_data = s_cb_data {
            reason: cbEndOfSimulation,
            cb_rtn: Some(rivet_end_of_sim),
            obj: std::ptr::null_mut(),
            time: std::ptr::null_mut(),
            value: std::ptr::null_mut(),
            index: 0,
            user_data: std::ptr::null_mut(),
        };
        // Xcelium does not deliver cbStartOfSimulation to late-loaded
        // libraries; cocotb uses cbAfterDelay(0) there (VpiCbHdl.cpp:236).
        let sim = SIM.with(|s| *s.borrow());
        static mut ZERO: s_vpi_time = s_vpi_time { type_: vpiSimTime, high: 0, low: 0, real: 0.0 };
        if sim == Sim::Xcelium {
            START.reason = cbAfterDelay;
            START.time = std::ptr::addr_of_mut!(ZERO);
        }
        if vpi_register_cb(std::ptr::addr_of_mut!(START)).is_null() {
            eprintln!("rivet: cannot register start-of-simulation callback");
        }
        if vpi_register_cb(std::ptr::addr_of_mut!(END)).is_null() {
            eprintln!("rivet: cannot register end-of-simulation callback");
        }
    }
}

unsafe extern "C" fn rivet_vpi_startup_routine() {
    let _ = catch_unwind(startup);
}

/// The table simulators scan when loading a VPI module.
#[no_mangle]
pub static vlog_startup_routines: [Option<unsafe extern "C" fn()>; 2] = [Some(rivet_vpi_startup_routine), None];

/// For simulators that need an explicit bootstrap symbol (Xcelium, CVC) and
/// for Verilator's `main`.
///
/// # Safety
/// Must be called from the simulator's thread, once, before any other
/// Rivet API.
#[no_mangle]
pub unsafe extern "C" fn vlog_startup_routines_bootstrap() {
    for r in vlog_startup_routines.iter().flatten() {
        r();
    }
}
