//! User-facing handles to design objects.

use crate::backend::{Action, Handle, ObjInfo, ObjKind, OwnedValue, Value};
use crate::error::{Error, Result};
use crate::runtime;
use crate::triggers::Edge;
use crate::value::{IntoLogicVec, LogicVec, Unresolved};
use std::fmt;

/// Any design object.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Object {
    h: Handle,
}

/// A hierarchical object: module, struct, package, generate array.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Module {
    h: Handle,
}

/// A value-carrying object: net, variable, parameter, array.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Signal {
    h: Handle,
}

fn info(h: Handle) -> ObjInfo {
    runtime::with(|rt| rt.backend.info(h).clone())
}

impl Object {
    pub fn from_handle(h: Handle) -> Object {
        Object { h }
    }

    pub fn handle(&self) -> Handle {
        self.h
    }

    pub fn info(&self) -> ObjInfo {
        info(self.h)
    }

    pub fn kind(&self) -> ObjKind {
        runtime::with(|rt| rt.backend.info(self.h).kind)
    }

    pub fn name(&self) -> String {
        runtime::with(|rt| rt.backend.info(self.h).name.clone())
    }

    pub fn path(&self) -> String {
        runtime::with(|rt| rt.backend.info(self.h).path.clone())
    }

    pub fn as_module(&self) -> Result<Module> {
        let i = self.info();
        if i.kind.is_hierarchy() {
            Ok(Module { h: self.h })
        } else {
            Err(crate::backend::BackendError::WrongKind { path: i.path, expected: "module", actual: i.kind }.into())
        }
    }

    pub fn as_signal(&self) -> Result<Signal> {
        let i = self.info();
        if i.kind.is_hierarchy() || i.kind == ObjKind::Unknown {
            Err(crate::backend::BackendError::WrongKind { path: i.path, expected: "signal", actual: i.kind }.into())
        } else {
            Ok(Signal { h: self.h })
        }
    }
}

impl Module {
    pub fn from_handle(h: Handle) -> Module {
        Module { h }
    }

    pub fn handle(&self) -> Handle {
        self.h
    }

    pub fn object(&self) -> Object {
        Object { h: self.h }
    }

    pub fn name(&self) -> String {
        self.object().name()
    }

    pub fn path(&self) -> String {
        self.object().path()
    }

    pub fn info(&self) -> ObjInfo {
        info(self.h)
    }

    /// Look up an immediate child by name.
    pub fn child(&self, name: &str) -> Result<Object> {
        let h = runtime::with(|rt| rt.backend.child_by_name(self.h, name))?;
        match h {
            Some(h) => Ok(Object { h }),
            None => Err(crate::backend::BackendError::NotFound(format!("{}.{}", self.path(), name)).into()),
        }
    }

    pub fn has_child(&self, name: &str) -> bool {
        runtime::with(|rt| rt.backend.child_by_name(self.h, name)).ok().flatten().is_some()
    }

    /// Child module by name.
    pub fn module(&self, name: &str) -> Result<Module> {
        self.child(name)?.as_module()
    }

    /// Child signal by name.
    pub fn signal(&self, name: &str) -> Result<Signal> {
        self.child(name)?.as_signal()
    }

    /// Element of a generate array or array of instances.
    pub fn index(&self, i: i64) -> Result<Object> {
        let h = runtime::with(|rt| rt.backend.child_by_index(self.h, i))?;
        match h {
            Some(h) => Ok(Object { h }),
            None => Err(crate::backend::BackendError::NotFound(format!("{}[{}]", self.path(), i)).into()),
        }
    }

    /// Resolve a dotted path relative to this module, e.g. `"u_core.alu.result"`.
    /// Bracketed indices are supported: `"mem[3]"`.
    pub fn path_object(&self, path: &str) -> Result<Object> {
        let mut cur = Object { h: self.h };
        for seg in path.split('.') {
            if seg.is_empty() {
                continue;
            }
            let (name, indices) = split_indices(seg)?;
            if !name.is_empty() {
                cur = cur.as_module()?.child(name)?;
            }
            for i in indices {
                cur = cur.as_module().or_else(|_| Err(Error::Msg(format!("{} is not indexable", cur.path()))))?.index(i)?;
            }
        }
        Ok(cur)
    }

    /// Resolve a dotted path to a signal.
    pub fn path_signal(&self, path: &str) -> Result<Signal> {
        self.path_object(path)?.as_signal()
    }

    /// Resolve a dotted path to a module.
    pub fn path_module(&self, path: &str) -> Result<Module> {
        self.path_object(path)?.as_module()
    }

    /// All immediate children.
    pub fn children(&self) -> Result<Vec<Object>> {
        let hs = runtime::with(|rt| rt.backend.children(self.h))?;
        Ok(hs.into_iter().map(|h| Object { h }).collect())
    }
}

fn split_indices(seg: &str) -> Result<(&str, Vec<i64>)> {
    let Some(open) = seg.find('[') else { return Ok((seg, Vec::new())) };
    let name = &seg[..open];
    let mut idx = Vec::new();
    for part in seg[open..].split('[').skip(1) {
        let Some(num) = part.strip_suffix(']') else {
            return Err(Error::Msg(format!("malformed index in {seg:?}")));
        };
        idx.push(num.parse::<i64>().map_err(|_| Error::Msg(format!("malformed index in {seg:?}")))?);
    }
    Ok((name, idx))
}

impl Signal {
    pub fn from_handle(h: Handle) -> Signal {
        Signal { h }
    }

    pub fn handle(&self) -> Handle {
        self.h
    }

    pub fn object(&self) -> Object {
        Object { h: self.h }
    }

    pub fn name(&self) -> String {
        self.object().name()
    }

    pub fn path(&self) -> String {
        self.object().path()
    }

    pub fn info(&self) -> ObjInfo {
        info(self.h)
    }

    pub fn kind(&self) -> ObjKind {
        self.object().kind()
    }

    pub fn width(&self) -> u32 {
        runtime::with(|rt| rt.backend.info(self.h).width)
    }

    pub fn is_const(&self) -> bool {
        runtime::with(|rt| rt.backend.info(self.h).is_const)
    }

    /// Read the value as a four-state vector.
    pub fn get(&self) -> LogicVec {
        let mut v = LogicVec::zeros(0);
        self.read_into(&mut v);
        v
    }

    /// Read into an existing buffer, avoiding allocation.
    pub fn read_into(&self, out: &mut LogicVec) {
        runtime::with(|rt| rt.backend.read_vec(self.h, out))
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", self.path()));
    }

    /// Read as an unsigned integer; `Err` if the value has X or Z bits.
    pub fn get_u64(&self) -> std::result::Result<u64, Unresolved> {
        self.get().to_u64()
    }

    /// Read as a signed integer; `Err` if the value has X or Z bits.
    pub fn get_i64(&self) -> std::result::Result<i64, Unresolved> {
        self.get().to_i64()
    }

    /// Read as an unsigned integer, resolving X/Z bits to 0.
    pub fn get_u64_lossy(&self) -> u64 {
        self.get().to_u64_lossy()
    }

    /// `true` if the scalar is `1`; errors on X/Z.
    pub fn is_high(&self) -> std::result::Result<bool, Unresolved> {
        Ok(self.get_u64()? != 0)
    }

    /// Read a real-valued object.
    pub fn get_real(&self) -> f64 {
        match self.read_raw() {
            OwnedValue::Real(r) => r,
            OwnedValue::Int(i) => i as f64,
            other => panic!("{} is not real: {other:?}", self.path()),
        }
    }

    /// Read a string-valued object.
    pub fn get_string(&self) -> String {
        match self.read_raw() {
            OwnedValue::Str(s) => s,
            other => panic!("{} is not a string: {other:?}", self.path()),
        }
    }

    pub fn read_raw(&self) -> OwnedValue {
        runtime::with(|rt| rt.backend.read(self.h)).unwrap_or_else(|e| panic!("cannot read {}: {e}", self.path()))
    }

    fn write_vec(&self, v: LogicVec, action: Action) {
        let path = || self.path();
        runtime::with(|rt| rt.schedule_write(self.h, OwnedValue::Vec(v), action))
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path()));
    }

    /// Inertial deposit, applied when the simulator next evaluates (or,
    /// on simulators that do not honour inertial writes, at the next
    /// ReadWrite phase). Reading back immediately returns the old value.
    pub fn set(&self, v: impl IntoLogicVec) {
        let w = self.width();
        self.write_vec(v.into_logic_vec(w), Action::Deposit);
    }

    /// Immediate deposit (`vpiNoDelay`).
    pub fn set_now(&self, v: impl IntoLogicVec) {
        let w = self.width();
        self.write_vec(v.into_logic_vec(w), Action::NoDelay);
    }

    /// Force the value, overriding drivers, until [`Signal::release`].
    pub fn force(&self, v: impl IntoLogicVec) {
        let w = self.width();
        self.write_vec(v.into_logic_vec(w), Action::Force);
    }

    /// Release a force.
    pub fn release(&self) {
        let cur = self.get();
        self.write_vec(cur, Action::Release);
    }

    pub fn set_real(&self, v: f64) {
        runtime::with(|rt| rt.schedule_write(self.h, OwnedValue::Real(v), Action::Deposit))
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", self.path()));
    }

    pub fn set_int(&self, v: i64) {
        runtime::with(|rt| rt.schedule_write(self.h, OwnedValue::Int(v), Action::Deposit))
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", self.path()));
    }

    pub fn set_string(&self, v: &str) {
        runtime::with(|rt| rt.schedule_write(self.h, OwnedValue::Str(v.to_string()), Action::Deposit))
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", self.path()));
    }

    /// Write any value with an explicit action.
    pub fn write(&self, v: Value<'_>, action: Action) -> Result<()> {
        let owned = match v {
            Value::Vec(x) => OwnedValue::Vec(x.clone()),
            Value::Int(x) => OwnedValue::Int(x),
            Value::Real(x) => OwnedValue::Real(x),
            Value::Str(x) => OwnedValue::Str(x.to_string()),
        };
        runtime::with(|rt| rt.schedule_write(self.h, owned, action)).map_err(Error::from)
    }

    /// Element of an unpacked array.
    pub fn index(&self, i: i64) -> Result<Signal> {
        let h = runtime::with(|rt| rt.backend.child_by_index(self.h, i))?;
        match h {
            Some(h) => Ok(Signal { h }),
            None => Err(crate::backend::BackendError::NotFound(format!("{}[{}]", self.path(), i)).into()),
        }
    }

    /// Member of a struct-valued object.
    pub fn member(&self, name: &str) -> Result<Signal> {
        let h = runtime::with(|rt| rt.backend.child_by_name(self.h, name))?;
        match h {
            Some(h) => Ok(Signal { h }),
            None => Err(crate::backend::BackendError::NotFound(format!("{}.{}", self.path(), name)).into()),
        }
    }

    /// Fires on the next transition to `1`.
    pub fn rising_edge(&self) -> Edge {
        crate::triggers::rising_edge(self.h)
    }

    /// Fires on the next transition to `0`.
    pub fn falling_edge(&self) -> Edge {
        crate::triggers::falling_edge(self.h)
    }

    /// Fires on the next value change.
    pub fn value_change(&self) -> Edge {
        crate::triggers::value_change(self.h)
    }
}

impl fmt::Debug for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Object({})", self.path())
    }
}
impl fmt::Debug for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Module({})", self.path())
    }
}
impl fmt::Debug for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signal({})", self.path())
    }
}
impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path())
    }
}
