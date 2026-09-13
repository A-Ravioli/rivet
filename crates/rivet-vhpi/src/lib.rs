//! VHPI backend: NVC, and (unverified) Questa, Riviera and Xcelium VHDL.
//!
//! VHPI is VHDL's procedural interface. Unlike VPI it has no packed
//! vectors, no inertial/immediate distinction on deposits, and types are
//! classified by the *content* of their enumeration rather than by kind,
//! so `std_logic`, `bit` and `boolean` are told apart by their literals
//! (`docs/design/00-cocotb-analysis.md` §3.6).
//!
//! The test crate is a `cdylib`; the exported `vhpi_startup_routines`
//! table makes a VHPI simulator load it (`nvc --load`, Questa
//! `-foreign vhpi_startup_routines_bootstrap`).

#![allow(non_upper_case_globals)]

pub mod ffi;

use ffi::*;
use rivet_core::backend::*;
use rivet_core::runtime::{self, Event};
use rivet_core::value::{Logic, LogicVec};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

/// Which simulator is hosting us.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Sim {
    Nvc,
    Questa,
    Riviera,
    Xcelium,
    Other,
}

/// How an enumeration type maps onto four-state logic.
#[derive(Debug, Default)]
pub struct EnumMap {
    /// Literal position to logic value, for logic-like types.
    to_logic: Vec<Logic>,
    /// Logic value to literal position, for writes.
    from_logic: HashMap<u8, u32>,
    /// Literal names, in position order.
    literals: Vec<String>,
    /// The type is `std_logic`, `std_ulogic` or `bit`.
    is_logic: bool,
    /// The type is `boolean`.
    is_boolean: bool,
    /// 256 literals: `character`.
    is_char: bool,
}

impl EnumMap {
    fn logic_at(&self, pos: u32) -> Logic {
        self.to_logic.get(pos as usize).copied().unwrap_or(Logic::X)
    }

    fn pos_of(&self, l: Logic) -> u32 {
        if let Some(&p) = self.from_logic.get(&(l as u8)) {
            return p;
        }
        // `bit` has no X or Z: fall back to the nearest defined value.
        match l {
            Logic::One => *self.from_logic.get(&(Logic::One as u8)).unwrap_or(&1),
            _ => *self.from_logic.get(&(Logic::Zero as u8)).unwrap_or(&0),
        }
    }
}

struct Entry {
    raw: vhpiHandleT,
    info: ObjInfo,
    children: Option<HashMap<String, Handle>>,
    /// Element (or scalar) enumeration map for logic-like objects.
    map: Option<Rc<EnumMap>>,
}

struct CbRec {
    id: CbId,
    kind: CbKind,
    cb_handle: vhpiHandleT,
    time: vhpiTimeT,
    data: vhpiCbDataT,
    removed: bool,
}

thread_local! {
    static CALLBACKS: RefCell<rivet_core::fxhash::FxHashMap<u64, Box<CbRec>>> = RefCell::new(Default::default());
    static NEXT_CB: RefCell<u64> = const { RefCell::new(1) };
}

/// The VHPI backend.
pub struct VhpiBackend {
    sim: Sim,
    tool: String,
    version: String,
    entries: Vec<Entry>,
    by_path: HashMap<String, Handle>,
    precision: i32,
    /// Enumeration maps, keyed by the type's full name.
    type_maps: HashMap<String, Rc<EnumMap>>,
    enum_buf: Vec<vhpiEnumT>,
    str_buf: Vec<u8>,
}

unsafe fn get_str(prop: i32, h: vhpiHandleT) -> String {
    cstr(vhpi_get_str(prop, h))
}

/// Strip the quotes some tools put around character literals (`'0'`), which
/// Aldec omits (cocotb VhpiImpl.cpp:174-201).
fn unquote(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('\'').unwrap_or(t);
    let t = t.strip_suffix('\'').unwrap_or(t);
    t.to_string()
}

impl VhpiBackend {
    pub fn new() -> VhpiBackend {
        // Tool identity: some tools answer on the null handle, others only
        // through the tool object.
        let (mut tool, mut version) =
            unsafe { (get_str(vhpiNameP, std::ptr::null_mut()), get_str(vhpiToolVersionP, std::ptr::null_mut())) };
        if tool.is_empty() {
            // NVC exposes the tool name through the root instance's design
            // unit chain; fall back to the environment it sets.
            tool = std::env::var("RIVET_VHPI_TOOL").unwrap_or_default();
        }
        if tool.is_empty() && !version.is_empty() {
            tool = version.clone();
        }
        if version.is_empty() {
            version = tool.clone();
        }
        let tl = tool.to_ascii_lowercase();
        let sim = if tl.contains("nvc") {
            Sim::Nvc
        } else if tl.contains("questa") || tl.contains("modelsim") {
            Sim::Questa
        } else if tl.contains("riviera") || tl.contains("aldec") || tl.contains("active-hdl") {
            Sim::Riviera
        } else if tl.contains("xcelium") || tl.contains("ncsim") {
            Sim::Xcelium
        } else {
            Sim::Other
        };
        // The resolution limit comes back as a physical value in
        // femtoseconds; Rivet wants the exponent of ten seconds.
        let fs = unsafe { vhpi_get_phys(vhpiResolutionLimitP, std::ptr::null_mut()) }.to_i64();
        let precision = if fs > 0 { (fs as f64).log10().round() as i32 - 15 } else { -15 };
        VhpiBackend {
            sim,
            tool,
            version,
            entries: Vec::new(),
            by_path: HashMap::new(),
            precision,
            type_maps: HashMap::new(),
            enum_buf: Vec::new(),
            str_buf: Vec::new(),
        }
    }

    pub fn sim(&self) -> Sim {
        self.sim
    }

    fn raw(&self, h: Handle) -> vhpiHandleT {
        self.entries[h.0 as usize].raw
    }

    /// The base type of an object, with the subtype fallback every tool
    /// needs somewhere (cocotb VhpiImpl.cpp:301-312).
    unsafe fn base_type(&self, raw: vhpiHandleT) -> vhpiHandleT {
        let t = vhpi_handle(vhpiBaseType, raw);
        if !t.is_null() {
            return t;
        }
        let st = vhpi_handle(vhpiSubtype_DEPRECATED, raw);
        if st.is_null() {
            return std::ptr::null_mut();
        }
        let t = vhpi_handle(vhpiBaseType, st);
        if t.is_null() {
            st
        } else {
            t
        }
    }

    unsafe fn elem_type(&self, ty: vhpiHandleT) -> vhpiHandleT {
        let e = vhpi_handle(vhpiElemType, ty);
        if !e.is_null() {
            return e;
        }
        vhpi_handle(vhpiElemSubtype_DEPRECATED, ty)
    }

    /// Build (or reuse) the enumeration map for an enum type declaration.
    unsafe fn enum_map(&mut self, ty: vhpiHandleT) -> Rc<EnumMap> {
        let key = get_str(vhpiFullNameP, ty);
        let key = if key.is_empty() { get_str(vhpiNameP, ty) } else { key };
        if let Some(m) = self.type_maps.get(&key) {
            return m.clone();
        }
        let mut literals = Vec::new();
        let it = vhpi_iterator(vhpiEnumLiterals, ty);
        if !it.is_null() {
            loop {
                let l = vhpi_scan(it);
                if l.is_null() {
                    break;
                }
                let mut name = get_str(vhpiStrValP, l);
                if name.is_empty() {
                    name = get_str(vhpiCaseNameP, l);
                }
                if name.is_empty() {
                    name = get_str(vhpiNameP, l);
                }
                literals.push(unquote(&name));
            }
        }
        let upper: Vec<String> = literals.iter().map(|s| s.to_ascii_uppercase()).collect();
        let logic_alphabet = ["U", "X", "0", "1", "Z", "W", "L", "H", "-"];
        let is_logic = !literals.is_empty()
            && upper.iter().all(|l| logic_alphabet.contains(&l.as_str()))
            && upper.iter().any(|l| l == "0" || l == "1");
        let is_boolean = upper.len() == 2 && upper[0] == "FALSE" && upper[1] == "TRUE";
        let is_char = upper.len() == 256;
        let mut to_logic = Vec::with_capacity(literals.len());
        let mut from_logic = HashMap::new();
        for (i, l) in upper.iter().enumerate() {
            let v = if is_logic {
                Logic::from_char(l.chars().next().unwrap_or('X')).unwrap_or(Logic::X)
            } else if is_boolean {
                if l == "TRUE" {
                    Logic::One
                } else {
                    Logic::Zero
                }
            } else {
                Logic::X
            };
            to_logic.push(v);
            from_logic.entry(v as u8).or_insert(i as u32);
        }
        let m = Rc::new(EnumMap { to_logic, from_logic, literals, is_logic, is_boolean, is_char });
        self.type_maps.insert(key, m.clone());
        m
    }

    fn intern(&mut self, raw: vhpiHandleT, path: Option<String>, fallback_name: &str) -> Handle {
        let name = unsafe {
            let n = get_str(vhpiCaseNameP, raw);
            let n = if n.is_empty() { get_str(vhpiNameP, raw) } else { n };
            if n.is_empty() {
                fallback_name.to_string()
            } else {
                n
            }
        };
        let path = path.unwrap_or_else(|| {
            let full = unsafe { get_str(vhpiFullCaseNameP, raw) };
            if full.is_empty() {
                name.clone()
            } else {
                full
            }
        });
        if let Some(&h) = self.by_path.get(&path) {
            unsafe { vhpi_release_handle(raw) };
            return h;
        }
        let (info, map) = self.classify(raw, name, path.clone());
        let h = Handle(self.entries.len() as u32);
        self.entries.push(Entry { raw, info, children: None, map });
        self.by_path.insert(path, h);
        h
    }

    fn classify(&mut self, raw: vhpiHandleT, name: String, path: String) -> (ObjInfo, Option<Rc<EnumMap>>) {
        let kind_k = unsafe { vhpi_get(vhpiKindP, raw) };
        let mut info = ObjInfo {
            kind: ObjKind::Unknown,
            name,
            path,
            width: 0,
            is_const: false,
            signed: false,
            range: None,
            type_name: String::new(),
        };
        let mut map = None;
        match kind_k {
            vhpiRootInstK | vhpiCompInstStmtK | vhpiBlockStmtK | vhpiForGenerateK | vhpiIfGenerateK => {
                info.kind = ObjKind::Module;
                info.type_name = "region".into();
            }
            vhpiPackInstK => {
                info.kind = ObjKind::Package;
                info.type_name = "package".into();
            }
            _ => {
                // A value-bearing object: classify by the content of its type.
                info.is_const = matches!(kind_k, vhpiGenericDeclK | vhpiConstDeclK);
                unsafe {
                    let ty = self.base_type(raw);
                    if ty.is_null() {
                        info.type_name = "unknown".into();
                    } else {
                        info.type_name = get_str(vhpiNameP, ty);
                        let tk = vhpi_get(vhpiKindP, ty);
                        let size = vhpi_get(vhpiSizeP, raw).max(0) as u32;
                        match tk {
                            vhpiEnumTypeDeclK => {
                                let m = self.enum_map(ty);
                                if m.is_logic || m.is_boolean {
                                    info.kind = ObjKind::Logic;
                                    info.width = 1;
                                } else if m.is_char {
                                    info.kind = ObjKind::Integer;
                                    info.width = 8;
                                } else {
                                    info.kind = ObjKind::Enum;
                                    let n = m.literals.len().max(1) as u32;
                                    info.width = (u32::BITS - (n - 1).leading_zeros()).max(1);
                                }
                                map = Some(m);
                            }
                            vhpiArrayTypeDeclK => {
                                let et = self.elem_type(ty);
                                let em = if et.is_null() { None } else { Some(self.enum_map(et)) };
                                let elem_logic = em.as_ref().map(|m| m.is_logic || m.is_boolean).unwrap_or(false);
                                let elem_char = em.as_ref().map(|m| m.is_char).unwrap_or(false);
                                if elem_logic {
                                    info.kind = ObjKind::LogicVec;
                                    info.width = size;
                                    map = em;
                                } else if elem_char {
                                    info.kind = ObjKind::String;
                                    info.width = size;
                                } else {
                                    info.kind = ObjKind::Array;
                                    info.width = size;
                                }
                                // Declared range, when the tool reports one.
                                let cons = vhpi_iterator(vhpiConstraints, ty);
                                if !cons.is_null() {
                                    let c = vhpi_scan(cons);
                                    if !c.is_null() {
                                        let l = vhpi_get(vhpiLeftBoundP, c) as i64;
                                        let r = vhpi_get(vhpiRightBoundP, c) as i64;
                                        info.range = Some((l, r));
                                    }
                                }
                            }
                            vhpiIntTypeDeclK => {
                                info.kind = ObjKind::Integer;
                                info.width = 32;
                                info.signed = true;
                            }
                            vhpiPhysTypeDeclK => {
                                info.kind = ObjKind::Integer;
                                info.width = 64;
                                info.signed = true;
                            }
                            vhpiFloatTypeDeclK => {
                                info.kind = ObjKind::Real;
                                info.width = 64;
                            }
                            vhpiRecordTypeDeclK => {
                                info.kind = ObjKind::Struct;
                            }
                            _ => {
                                info.kind = ObjKind::Unknown;
                            }
                        }
                    }
                }
            }
        }
        (info, map)
    }

    fn check_error(&self, what: &str) -> Result<()> {
        let mut e = vhpiErrorInfoT {
            severity: 0,
            message: std::ptr::null_mut(),
            str_: std::ptr::null_mut(),
            file: std::ptr::null_mut(),
            line: 0,
        };
        let got = unsafe { vhpi_check_error(&mut e) };
        if got != 0 && !e.message.is_null() {
            let msg = unsafe { std::ffi::CStr::from_ptr(e.message).to_string_lossy().into_owned() };
            return Err(BackendError::Sim(format!("{what}: {msg}")));
        }
        Ok(())
    }

    /// Iterate one relation, calling `f` for every object it yields.
    unsafe fn for_each(&mut self, rel: i32, parent: vhpiHandleT, mut f: impl FnMut(&mut Self, vhpiHandleT)) {
        let it = vhpi_iterator(rel, parent);
        if it.is_null() {
            return;
        }
        loop {
            let h = vhpi_scan(it);
            if h.is_null() {
                break;
            }
            f(self, h);
        }
    }

    /// Read a vector through the logic-vector format, falling back to a
    /// binary string for tools that refuse it. (Writes never fall back:
    /// NVC makes an unsupported put format a fatal error.)
    fn read_logic_vec(&mut self, h: Handle, out: &mut LogicVec) -> Result<()> {
        let raw = self.raw(h);
        let width = self.entries[h.0 as usize].info.width;
        let map = self.entries[h.0 as usize].map.clone();
        out.resize(width);
        self.enum_buf.clear();
        self.enum_buf.resize(width as usize, 0);
        let mut v = vhpiValueT {
            format: vhpiLogicVecVal,
            bufSize: (width as usize) * std::mem::size_of::<vhpiEnumT>(),
            numElems: width as i32,
            unit: vhpiPhysT::default(),
            value: vhpiValueUnion { enumvs: self.enum_buf.as_mut_ptr() },
        };
        let rc = unsafe { vhpi_get_value(raw, &mut v) };
        if rc == 0 {
            let m = map.as_ref();
            for i in 0..width {
                let pos = self.enum_buf[i as usize];
                let l = m.map(|m| m.logic_at(pos)).unwrap_or(Logic::X);
                // VHDL arrays are reported left to right; bit 0 is the last.
                out.set_bit(width - 1 - i, l);
            }
            return Ok(());
        }
        self.read_binstr(raw, width, out)
    }

    fn read_binstr(&mut self, raw: vhpiHandleT, width: u32, out: &mut LogicVec) -> Result<()> {
        self.str_buf.clear();
        self.str_buf.resize(width as usize + 2, 0);
        let mut v = vhpiValueT {
            format: vhpiBinStrVal,
            bufSize: self.str_buf.len(),
            numElems: 0,
            unit: vhpiPhysT::default(),
            value: vhpiValueUnion { str_: self.str_buf.as_mut_ptr() },
        };
        let rc = unsafe { vhpi_get_value(raw, &mut v) };
        if rc != 0 {
            self.check_error("vhpi_get_value")?;
            return Err(BackendError::Sim("vhpi_get_value failed".into()));
        }
        let s = unsafe { cstr(self.str_buf.as_ptr()) };
        out.resize(width);
        let chars: Vec<char> = s.chars().collect();
        for (i, c) in chars.iter().rev().enumerate() {
            if i as u32 >= width {
                break;
            }
            out.set_bit(i as u32, Logic::from_char(*c).unwrap_or(Logic::X));
        }
        Ok(())
    }

    fn write_logic_vec(&mut self, h: Handle, val: &LogicVec, mode: i32) -> Result<()> {
        let raw = self.raw(h);
        let info_width = self.entries[h.0 as usize].info.width;
        let kind = self.entries[h.0 as usize].info.kind;
        let map = self.entries[h.0 as usize].map.clone();
        let Some(m) = map else {
            return Err(BackendError::Unsupported(format!("write to {}", self.entries[h.0 as usize].info.path)));
        };
        if kind == ObjKind::Logic {
            let pos = m.pos_of(val.bit(0));
            let mut v = vhpiValueT {
                // A scalar of a logic type takes vhpiLogicVal. NVC accepts
                // vhpiEnumVal without complaint and then leaves the signal at
                // its left-most literal ('U'), so the format matters.
                format: vhpiLogicVal,
                bufSize: std::mem::size_of::<vhpiEnumT>(),
                numElems: 1,
                unit: vhpiPhysT::default(),
                value: vhpiValueUnion { enumv: pos },
            };
            let rc = unsafe { vhpi_put_value(raw, &mut v, mode) };
            if rc != 0 {
                self.check_error("vhpi_put_value")?;
                return Err(BackendError::Sim("vhpi_put_value failed".into()));
            }
            log::trace!("put {} = literal {pos} (mode {mode})", self.entries[h.0 as usize].info.path);
            return Ok(());
        }
        let width = info_width;
        self.enum_buf.clear();
        self.enum_buf.reserve(width as usize);
        for i in 0..width {
            // Left to right, so the most significant bit goes first.
            self.enum_buf.push(m.pos_of(val.bit(width - 1 - i)));
        }
        let mut v = vhpiValueT {
            format: vhpiLogicVecVal,
            bufSize: (width as usize) * std::mem::size_of::<vhpiEnumT>(),
            numElems: width as i32,
            unit: vhpiPhysT::default(),
            value: vhpiValueUnion { enumvs: self.enum_buf.as_mut_ptr() },
        };
        let rc = unsafe { vhpi_put_value(raw, &mut v, mode) };
        if rc != 0 {
            self.check_error("vhpi_put_value")?;
            return Err(BackendError::Sim("vhpi_put_value failed".into()));
        }
        Ok(())
    }
}

impl Default for VhpiBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Relations that make up a region's children.
const REGION_CHILDREN: &[i32] = &[
    vhpiPortDecls,
    vhpiSigDecls,
    vhpiVarDecls,
    vhpiGenericDecls,
    vhpiConstDecls,
    vhpiInternalRegions,
    vhpiCompInstStmts,
    vhpiBlockStmts,
];

impl Backend for VhpiBackend {
    fn name(&self) -> &str {
        match self.sim {
            Sim::Nvc => "nvc",
            Sim::Questa => "questa",
            Sim::Riviera => "riviera",
            Sim::Xcelium => "xcelium",
            Sim::Other => "vhpi",
        }
    }

    fn version(&self) -> String {
        format!("{} {}", self.tool, self.version).trim().to_string()
    }

    fn caps(&self) -> Capabilities {
        Capabilities {
            // VHPI has no inertial/immediate distinction: both deposit
            // modes propagate, so the runtime buffers writes as it does
            // everywhere else (cocotb VhpiSignal.cpp:35-52).
            trusts_inertial_writes: false,
            // The Rep* phase callbacks repeat; the handler removes them.
            remove_fired_callbacks: true,
            four_state: true,
            supports_force: true,
        }
    }

    fn precision(&self) -> i32 {
        self.precision
    }

    fn now(&self) -> u64 {
        let mut t = vhpiTimeT::default();
        unsafe { vhpi_get_time(&mut t, std::ptr::null_mut()) };
        t.to_u64()
    }

    fn root(&mut self, name: Option<&str>) -> Result<Handle> {
        let raw = unsafe { vhpi_handle(vhpiRootInst, std::ptr::null_mut()) };
        if raw.is_null() {
            return Err(BackendError::NotFound("no root instance".into()));
        }
        let actual = unsafe {
            let n = get_str(vhpiCaseNameP, raw);
            if n.is_empty() {
                get_str(vhpiNameP, raw)
            } else {
                n
            }
        };
        if let Some(want) = name {
            // NVC upper-cases names (cocotb VhpiImpl.cpp:274-285, nvc#723).
            if !actual.eq_ignore_ascii_case(want) {
                log::debug!("rivet: root is {actual}, test asked for {want}");
            }
        }
        Ok(self.intern(raw, Some(actual.clone()), &actual))
    }

    fn child_by_name(&mut self, parent: Handle, name: &str) -> Result<Option<Handle>> {
        if let Some(ch) = &self.entries[parent.0 as usize].children {
            if let Some(&h) = ch.get(name) {
                return Ok(Some(h));
            }
            if let Some((_, &h)) = ch.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
                return Ok(Some(h));
            }
        }
        // Discover the region's children once, then match case-insensitively.
        let _ = self.children(parent)?;
        if let Some(ch) = &self.entries[parent.0 as usize].children {
            if let Some((_, &h)) = ch.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
                return Ok(Some(h));
            }
        }
        // A VHDL for-generate has no parent object: each element is its own
        // region named `LABEL(i)`. Synthesise the array, as the VPI backend
        // does for Verilog generate blocks.
        let ppath = self.entries[parent.0 as usize].info.path.clone();
        let prefix = format!("{}(", name.to_ascii_lowercase());
        let is_gen = self.entries[parent.0 as usize]
            .children
            .as_ref()
            .map(|ch| ch.keys().any(|k| k.to_ascii_lowercase().starts_with(&prefix)))
            .unwrap_or(false);
        if is_gen {
            let path = format!("{ppath}.{name}");
            let praw = self.raw(parent);
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
                map: None,
            });
            self.by_path.insert(path, h);
            self.entries[parent.0 as usize].children.get_or_insert_with(HashMap::new).insert(name.to_string(), h);
            return Ok(Some(h));
        }

        // Last resort: ask the simulator by fully qualified name. VHDL uses
        // ':' separators and a leading ':' disambiguates library objects
        // (cocotb VhpiImpl.cpp:884-982).
        let vhdl_path = format!(":{}:{name}", ppath.replace('.', ":"));
        let c = CString::new(vhdl_path).map_err(|_| BackendError::NotFound(name.into()))?;
        let raw = unsafe { vhpi_handle_by_name(c.as_ptr(), std::ptr::null_mut()) };
        if raw.is_null() {
            return Ok(None);
        }
        let h = self.intern(raw, Some(format!("{ppath}.{name}")), name);
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
        let praw = self.raw(parent);
        let ppath = self.entries[parent.0 as usize].info.path.clone();
        // Generate arrays are a fiction: find the element region by name.
        if self.entries[parent.0 as usize].info.kind == ObjKind::GenArray {
            let label = self.entries[parent.0 as usize].info.name.clone();
            // The parent entry shares the enclosing region's raw handle, so
            // its children are the enclosing region's children.
            let want = format!("{}({index})", label.to_ascii_lowercase());
            let mut found = None;
            let mut i = 0usize;
            unsafe {
                self.for_each(vhpiInternalRegions, praw, |s2, h| {
                    let mut n = get_str(vhpiCaseNameP, h);
                    if n.is_empty() {
                        n = get_str(vhpiNameP, h);
                    }
                    let leaf = n.rsplit([':', '.']).next().unwrap_or(&n).to_ascii_lowercase();
                    if leaf == want && found.is_none() {
                        found = Some(s2.intern(h, Some(format!("{ppath}[{index}]")), &format!("[{index}]")));
                    } else {
                        vhpi_release_handle(h);
                    }
                    i += 1;
                })
            };
            if let Some(h) = found {
                self.entries[parent.0 as usize].children.get_or_insert_with(HashMap::new).insert(key, h);
                return Ok(Some(h));
            }
            return Ok(None);
        }
        let mut raw = unsafe { vhpi_handle_by_index(vhpiIndexedNames, praw, index as i32) };
        if raw.is_null() {
            // Some tools only answer a linear scan of vhpiIndexedNames
            // (cocotb VhpiImpl.cpp:833-858).
            let mut found = std::ptr::null_mut();
            let mut i = 0i64;
            unsafe {
                self.for_each(vhpiIndexedNames, praw, |_s, h| {
                    if i == index {
                        found = h;
                    }
                    i += 1;
                })
            };
            raw = found;
        }
        if raw.is_null() {
            return Ok(None);
        }
        let h = self.intern(raw, Some(format!("{ppath}[{index}]")), &key);
        self.entries[parent.0 as usize].children.get_or_insert_with(HashMap::new).insert(key, h);
        Ok(Some(h))
    }

    fn children(&mut self, parent: Handle) -> Result<Vec<Handle>> {
        let praw = self.raw(parent);
        let ppath = self.entries[parent.0 as usize].info.path.clone();
        let kind = self.entries[parent.0 as usize].info.kind;
        let rels: &[i32] = match kind {
            // Record members are selected names; array elements are indexed.
            ObjKind::Struct => &[vhpiSelectedNames],
            ObjKind::Array => &[vhpiIndexedNames],
            _ => REGION_CHILDREN,
        };
        let mut raws: Vec<(String, vhpiHandleT)> = Vec::new();
        for &rel in rels {
            unsafe {
                self.for_each(rel, praw, |_s, h| {
                    let k = vhpi_get(vhpiKindP, h);
                    // Processes and subprogram bodies are not design data.
                    if k == vhpiProcessStmtK || k == vhpiSubpBodyK {
                        vhpi_release_handle(h);
                        return;
                    }
                    let mut name = get_str(vhpiCaseNameP, h);
                    if name.is_empty() {
                        name = get_str(vhpiNameP, h);
                    }
                    if name.is_empty() {
                        vhpi_release_handle(h);
                        return;
                    }
                    // Selected and indexed names come back fully qualified.
                    let leaf = name.rsplit([':', '.']).next().unwrap_or(&name).to_string();
                    raws.push((leaf, h));
                })
            }
        }
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (name, raw) in raws {
            let h = self.intern(raw, Some(format!("{ppath}.{name}")), &name);
            if seen.insert(h) {
                out.push(h);
                self.entries[parent.0 as usize].children.get_or_insert_with(HashMap::new).insert(name, h);
            }
        }
        Ok(out)
    }

    fn info(&self, h: Handle) -> &ObjInfo {
        &self.entries[h.0 as usize].info
    }

    fn read(&mut self, h: Handle) -> Result<OwnedValue> {
        let kind = self.entries[h.0 as usize].info.kind;
        let raw = self.raw(h);
        match kind {
            ObjKind::Integer | ObjKind::Enum if self.entries[h.0 as usize].map.is_none() => {
                let mut v = vhpiValueT { format: vhpiIntVal, ..Default::default() };
                let rc = unsafe { vhpi_get_value(raw, &mut v) };
                if rc != 0 {
                    self.check_error("vhpi_get_value")?;
                }
                Ok(OwnedValue::Int(unsafe { v.value.intg } as i64))
            }
            ObjKind::Real => {
                let mut v = vhpiValueT { format: vhpiRealVal, ..Default::default() };
                let rc = unsafe { vhpi_get_value(raw, &mut v) };
                if rc != 0 {
                    self.check_error("vhpi_get_value")?;
                }
                Ok(OwnedValue::Real(unsafe { v.value.real }))
            }
            ObjKind::String => {
                let width = self.entries[h.0 as usize].info.width.max(1);
                self.str_buf.clear();
                self.str_buf.resize(width as usize + 2, 0);
                let mut v = vhpiValueT {
                    format: vhpiStrVal,
                    bufSize: self.str_buf.len(),
                    numElems: width as i32,
                    unit: vhpiPhysT::default(),
                    value: vhpiValueUnion { str_: self.str_buf.as_mut_ptr() },
                };
                let rc = unsafe { vhpi_get_value(raw, &mut v) };
                if rc != 0 {
                    self.check_error("vhpi_get_value")?;
                }
                Ok(OwnedValue::Str(unsafe { cstr(self.str_buf.as_ptr()) }))
            }
            _ => {
                let mut out = LogicVec::zeros(self.entries[h.0 as usize].info.width.max(1));
                self.read_vec(h, &mut out)?;
                Ok(OwnedValue::Vec(out))
            }
        }
    }

    fn read_vec(&mut self, h: Handle, out: &mut LogicVec) -> Result<()> {
        let kind = self.entries[h.0 as usize].info.kind;
        let raw = self.raw(h);
        match kind {
            ObjKind::Logic => {
                let map = self.entries[h.0 as usize].map.clone();
                let mut v = vhpiValueT { format: vhpiLogicVal, numElems: 1, ..Default::default() };
                let rc = unsafe { vhpi_get_value(raw, &mut v) };
                if rc != 0 {
                    self.check_error("vhpi_get_value")?;
                    return Err(BackendError::Sim("vhpi_get_value failed".into()));
                }
                let pos = unsafe { v.value.enumv };
                out.resize(1);
                out.set_bit(0, map.map(|m| m.logic_at(pos)).unwrap_or(Logic::X));
                Ok(())
            }
            ObjKind::LogicVec => self.read_logic_vec(h, out),
            ObjKind::Enum => {
                let width = self.entries[h.0 as usize].info.width.max(1);
                let mut v = vhpiValueT { format: vhpiEnumVal, numElems: 1, ..Default::default() };
                let rc = unsafe { vhpi_get_value(raw, &mut v) };
                if rc != 0 {
                    self.check_error("vhpi_get_value")?;
                    return Err(BackendError::Sim("vhpi_get_value failed".into()));
                }
                *out = LogicVec::from_u64(width, unsafe { v.value.enumv } as u64);
                Ok(())
            }
            ObjKind::Integer => {
                let width = self.entries[h.0 as usize].info.width.max(1);
                let mut v = vhpiValueT { format: vhpiIntVal, ..Default::default() };
                let rc = unsafe { vhpi_get_value(raw, &mut v) };
                if rc != 0 {
                    self.check_error("vhpi_get_value")?;
                    return Err(BackendError::Sim("vhpi_get_value failed".into()));
                }
                let iv = unsafe { v.value.intg } as i64;
                *out = LogicVec::from_i64(width, iv);
                Ok(())
            }
            other => Err(BackendError::WrongKind {
                path: self.entries[h.0 as usize].info.path.clone(),
                expected: "a value object",
                actual: other,
            }),
        }
    }

    fn write(&mut self, h: Handle, v: Value<'_>, action: Action) -> Result<()> {
        let info = &self.entries[h.0 as usize].info;
        if info.is_const {
            return Err(BackendError::Unsupported(format!("{} is a constant", info.path)));
        }
        let kind = info.kind;
        let width = info.width.max(1);
        let raw = self.raw(h);
        // VHPI has no immediate/inertial split: both deposit modes
        // propagate (cocotb VhpiSignal.cpp:35-52).
        let mode = match action {
            // Only the *Propagate modes exist in practice: NVC makes
            // vhpiDeposit and vhpiForce a fatal error.
            Action::Deposit | Action::NoDelay => vhpiDepositPropagate,
            Action::Force => vhpiForcePropagate,
            Action::Release => vhpiRelease,
        };
        if action == Action::Release {
            let mut val = vhpiValueT { format: vhpiObjTypeVal, ..Default::default() };
            let rc = unsafe { vhpi_put_value(raw, &mut val, vhpiRelease) };
            if rc != 0 {
                self.check_error("vhpi_put_value(release)")?;
            }
            return Ok(());
        }
        match (kind, v) {
            (ObjKind::Logic | ObjKind::LogicVec, Value::Vec(lv)) => self.write_logic_vec(h, lv, mode),
            (ObjKind::Logic | ObjKind::LogicVec, Value::Int(i)) => {
                let lv = LogicVec::from_i64(width, i);
                self.write_logic_vec(h, &lv, mode)
            }
            (ObjKind::Enum, v) => {
                let pos = match v {
                    Value::Int(i) => i as u32,
                    Value::Vec(lv) => lv.to_u64_lossy() as u32,
                    Value::Real(r) => r as u32,
                    Value::Str(s) => {
                        // Accept the literal's name as well as its position.
                        let m = self.entries[h.0 as usize].map.clone();
                        match m.as_ref().and_then(|m| m.literals.iter().position(|l| l.eq_ignore_ascii_case(s))) {
                            Some(p) => p as u32,
                            None => {
                                return Err(BackendError::Unsupported(format!("{s:?} is not a literal of this type")))
                            }
                        }
                    }
                };
                let mut val = vhpiValueT {
                    format: vhpiEnumVal,
                    bufSize: std::mem::size_of::<vhpiEnumT>(),
                    numElems: 1,
                    unit: vhpiPhysT::default(),
                    value: vhpiValueUnion { enumv: pos },
                };
                let rc = unsafe { vhpi_put_value(raw, &mut val, mode) };
                if rc != 0 {
                    self.check_error("vhpi_put_value")?;
                    return Err(BackendError::Sim("vhpi_put_value failed".into()));
                }
                Ok(())
            }
            (ObjKind::Integer, v) => {
                let iv = match v {
                    Value::Int(i) => i,
                    Value::Vec(lv) => lv.to_u64_lossy() as i64,
                    Value::Real(r) => r as i64,
                    Value::Str(_) => {
                        return Err(BackendError::Unsupported("string into an integer".into()));
                    }
                };
                let mut val = vhpiValueT {
                    format: vhpiIntVal,
                    bufSize: 0,
                    numElems: 0,
                    unit: vhpiPhysT::default(),
                    value: vhpiValueUnion { intg: iv as i32 },
                };
                let rc = unsafe { vhpi_put_value(raw, &mut val, mode) };
                if rc != 0 {
                    self.check_error("vhpi_put_value")?;
                    return Err(BackendError::Sim("vhpi_put_value failed".into()));
                }
                Ok(())
            }
            (ObjKind::Real, v) => {
                let r = match v {
                    Value::Real(r) => r,
                    Value::Int(i) => i as f64,
                    Value::Vec(lv) => lv.to_u64_lossy() as f64,
                    Value::Str(_) => return Err(BackendError::Unsupported("string into a real".into())),
                };
                let mut val = vhpiValueT {
                    format: vhpiRealVal,
                    bufSize: 0,
                    numElems: 0,
                    unit: vhpiPhysT::default(),
                    value: vhpiValueUnion { real: r },
                };
                let rc = unsafe { vhpi_put_value(raw, &mut val, mode) };
                if rc != 0 {
                    self.check_error("vhpi_put_value")?;
                    return Err(BackendError::Sim("vhpi_put_value failed".into()));
                }
                Ok(())
            }
            (ObjKind::String, Value::Str(s)) => {
                let c = CString::new(s).map_err(|_| BackendError::Unsupported("NUL in string".into()))?;
                let mut bytes: Vec<u8> = c.as_bytes_with_nul().to_vec();
                let mut val = vhpiValueT {
                    format: vhpiStrVal,
                    bufSize: bytes.len(),
                    numElems: (bytes.len() - 1) as i32,
                    unit: vhpiPhysT::default(),
                    value: vhpiValueUnion { str_: bytes.as_mut_ptr() },
                };
                let rc = unsafe { vhpi_put_value(raw, &mut val, mode) };
                if rc != 0 {
                    self.check_error("vhpi_put_value")?;
                    return Err(BackendError::Sim("vhpi_put_value failed".into()));
                }
                Ok(())
            }
            (k, _) => Err(BackendError::Unsupported(format!("write to {k:?}"))),
        }
    }

    fn register(&mut self, kind: CbKind) -> Result<CbId> {
        let id = CbId(NEXT_CB.with(|n| {
            let mut n = n.borrow_mut();
            let v = *n;
            *n += 1;
            v
        }));
        let (reason, obj) = match kind {
            CbKind::ValueChange(h) => (vhpiCbValueChange, self.raw(h)),
            CbKind::AfterDelay(_) => (vhpiCbAfterDelay, std::ptr::null_mut()),
            // The one-shot phase callbacks are optional in VHPI; the
            // repetitive ones are what tools implement, so register those
            // and remove them when they fire (cocotb VhpiCbHdl.cpp).
            CbKind::ReadWrite => (vhpiCbRepLastKnownDeltaCycle, std::ptr::null_mut()),
            CbKind::ReadOnly => (vhpiCbRepEndOfTimeStep, std::ptr::null_mut()),
            CbKind::NextTimeStep => (vhpiCbRepNextTimeStep, std::ptr::null_mut()),
        };
        let mut rec = Box::new(CbRec {
            id,
            kind,
            cb_handle: std::ptr::null_mut(),
            time: match kind {
                CbKind::AfterDelay(steps) => vhpiTimeT::from_u64(steps),
                _ => vhpiTimeT::default(),
            },
            data: vhpiCbDataT {
                reason,
                cb_rtn: Some(rivet_vhpi_callback),
                obj,
                time: std::ptr::null_mut(),
                value: std::ptr::null_mut(),
                user_data: std::ptr::null_mut(),
            },
            removed: false,
        });
        let p: *mut CbRec = &mut *rec;
        unsafe {
            (*p).data.user_data = p as *mut c_void;
            if matches!(kind, CbKind::AfterDelay(_)) {
                (*p).data.time = &mut (*p).time;
            }
            let h = vhpi_register_cb(&mut (*p).data, vhpiReturnCb);
            if h.is_null() {
                return Err(BackendError::Sim(format!("vhpi_register_cb failed for {kind:?}")));
            }
            (*p).cb_handle = h;
        }
        CALLBACKS.with(|c| c.borrow_mut().insert(id.0, rec));
        Ok(id)
    }

    fn remove(&mut self, id: CbId) -> Result<()> {
        let rec = CALLBACKS.with(|c| c.borrow_mut().remove(&id.0));
        if let Some(mut rec) = rec {
            rec.removed = true;
            if !rec.cb_handle.is_null() {
                unsafe { vhpi_remove_cb(rec.cb_handle) };
                rec.cb_handle = std::ptr::null_mut();
            }
        }
        Ok(())
    }

    fn enum_literals(&mut self, h: Handle) -> Option<Vec<String>> {
        let e = &self.entries[h.0 as usize];
        match (&e.map, e.info.kind) {
            (Some(m), ObjKind::Enum) | (Some(m), ObjKind::Logic) if !m.literals.is_empty() => Some(m.literals.clone()),
            _ => None,
        }
    }

    fn finish(&mut self) {
        unsafe { vhpi_control(vhpiFinish, 0) };
    }

    fn waves(&mut self, _cmd: WaveCmd) -> Result<()> {
        // NVC dumps waveforms with `--wave` for the whole run; there is no
        // VHPI call to start or stop one.
        Err(BackendError::Unsupported("waveform control on VHPI".into()))
    }

    fn argv(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The single C entry point for every callback we register.
unsafe extern "C" fn rivet_vhpi_callback(cb: *const vhpiCbDataT) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if cb.is_null() {
            return;
        }
        let p = (*cb).user_data as *mut CbRec;
        if p.is_null() {
            return;
        }
        let (id, kind, removed) = ((*p).id, (*p).kind, (*p).removed);
        if removed {
            return;
        }
        let one_shot = !matches!(kind, CbKind::ValueChange(_));
        let event = match kind {
            CbKind::ValueChange(h) => Event::ValueChange(h),
            CbKind::AfterDelay(_) => Event::Timer(id),
            CbKind::ReadWrite => Event::ReadWrite,
            CbKind::ReadOnly => Event::ReadOnly,
            CbKind::NextTimeStep => Event::NextTimeStep,
        };
        // Phase callbacks are registered in their repetitive form, so a
        // one-shot must be removed here or it fires every cycle.
        let rec = if one_shot { CALLBACKS.with(|c| c.borrow_mut().remove(&id.0)) } else { None };
        if let Some(rec) = &rec {
            if !rec.cb_handle.is_null() {
                vhpi_remove_cb(rec.cb_handle);
            }
        }
        runtime::dispatch(event);
        drop(rec);
    }));
}

unsafe extern "C" fn rivet_start_of_sim(_cb: *const vhpiCbDataT) {
    let res = catch_unwind(|| runtime::dispatch(Event::StartOfSim));
    if let Err(p) = res {
        eprintln!("rivet: panic at start of simulation: {}", runtime::panic_message(&p));
        vhpi_control(vhpiFinish, 1);
    }
}

static EXIT_CODE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Exit code decided by the regression, available after end of simulation.
pub fn exit_code() -> i32 {
    EXIT_CODE.load(std::sync::atomic::Ordering::Relaxed)
}

unsafe extern "C" fn rivet_end_of_sim(_cb: *const vhpiCbDataT) {
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
}

/// Initialise the runtime with a VHPI backend and hook start and end of
/// simulation. Called from `vhpi_startup_routines`.
pub fn startup() {
    rivet_core::log::init();
    let backend = VhpiBackend::new();
    log::debug!("rivet VHPI backend on {} ({:?})", backend.version(), backend.sim());
    runtime::init(Box::new(backend));
    rivet_core::test::install_default_entry();
    unsafe {
        static mut START: vhpiCbDataT = vhpiCbDataT {
            reason: vhpiCbStartOfSimulation,
            cb_rtn: Some(rivet_start_of_sim),
            obj: std::ptr::null_mut(),
            time: std::ptr::null_mut(),
            value: std::ptr::null_mut(),
            user_data: std::ptr::null_mut(),
        };
        static mut END: vhpiCbDataT = vhpiCbDataT {
            reason: vhpiCbEndOfSimulation,
            cb_rtn: Some(rivet_end_of_sim),
            obj: std::ptr::null_mut(),
            time: std::ptr::null_mut(),
            value: std::ptr::null_mut(),
            user_data: std::ptr::null_mut(),
        };
        if vhpi_register_cb(std::ptr::addr_of_mut!(START), vhpiReturnCb).is_null() {
            eprintln!("rivet: cannot register start-of-simulation callback");
        }
        if vhpi_register_cb(std::ptr::addr_of_mut!(END), vhpiReturnCb).is_null() {
            eprintln!("rivet: cannot register end-of-simulation callback");
        }
    }
}

unsafe extern "C" fn rivet_vhpi_startup_routine() {
    let _ = catch_unwind(startup);
}

/// The table a VHPI simulator scans when loading the library.
#[no_mangle]
pub static vhpi_startup_routines: [Option<unsafe extern "C" fn()>; 2] = [Some(rivet_vhpi_startup_routine), None];

/// Bootstrap symbol for tools that need an explicit `-foreign` entry point
/// (Questa, Riviera).
///
/// # Safety
/// Must be called from the simulator's thread, once, before any other
/// Rivet API.
#[no_mangle]
pub unsafe extern "C" fn vhpi_startup_routines_bootstrap() {
    for r in vhpi_startup_routines.iter().flatten() {
        r();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_are_stripped_from_literals() {
        assert_eq!(unquote("'0'"), "0");
        assert_eq!(unquote("1"), "1");
        assert_eq!(unquote(" 'Z' "), "Z");
    }

    #[test]
    fn enum_map_maps_std_logic_positions() {
        // std_logic is (U, X, 0, 1, Z, W, L, H, -).
        let lits = ["U", "X", "0", "1", "Z", "W", "L", "H", "-"];
        let mut to_logic = Vec::new();
        let mut from_logic = HashMap::new();
        for (i, l) in lits.iter().enumerate() {
            let v = Logic::from_char(l.chars().next().unwrap()).unwrap_or(Logic::X);
            to_logic.push(v);
            from_logic.entry(v as u8).or_insert(i as u32);
        }
        let m = EnumMap {
            to_logic,
            from_logic,
            literals: lits.iter().map(|s| s.to_string()).collect(),
            is_logic: true,
            is_boolean: false,
            is_char: false,
        };
        assert_eq!(m.logic_at(2), Logic::Zero);
        assert_eq!(m.logic_at(3), Logic::One);
        assert_eq!(m.logic_at(4), Logic::Z);
        assert_eq!(m.logic_at(1), Logic::X);
        // 'L' and 'H' resolve to 0 and 1.
        assert_eq!(m.logic_at(6), Logic::Zero);
        assert_eq!(m.logic_at(7), Logic::One);
        // Writes pick the canonical position, not the weak one.
        assert_eq!(m.pos_of(Logic::Zero), 2);
        assert_eq!(m.pos_of(Logic::One), 3);
        assert_eq!(m.pos_of(Logic::Z), 4);
    }

    #[test]
    fn time_round_trips() {
        let t = vhpiTimeT::from_u64(0x1234_5678_9abc);
        assert_eq!(t.to_u64(), 0x1234_5678_9abc);
    }
}
