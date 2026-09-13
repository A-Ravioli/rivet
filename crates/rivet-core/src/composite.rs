//! Running one testbench across two procedural interfaces.
//!
//! A mixed-language design is one simulation kernel seen through two
//! interfaces: the Verilog half through VPI, the VHDL half through VHPI (or
//! FLI). Neither interface can see the other's objects — `vpi_handle_by_name`
//! does not find a VHDL signal, and `vhpi_handle_by_name` does not find a
//! Verilog one — so a harness that wants the whole hierarchy has to hold both
//! and route each lookup to the interface that owns the object. cocotb does
//! this inside the GPI with an implementation registry; [`CompositeBackend`]
//! does it here.
//!
//! # Identity
//!
//! Handles and callback ids are per-backend indices, so the composite tags
//! them with the index of the backend that issued them: the top
//! [`TAG_BITS`] bits of a [`Handle`], the top eight of a [`CbId`]. Tag 0 is
//! the primary, so its handles pass through unchanged. Everything above the
//! backend boundary — the runtime, triggers, `Module`, `Signal` — keeps
//! treating a handle as an opaque integer.
//!
//! Events travel the other way: a backend dispatches them itself, so a
//! secondary has to tag what it dispatches. That is what
//! [`Backend::set_event_tag`] is for, and [`CompositeBackend::new`] refuses
//! a secondary that does not support it rather than silently mixing up two
//! backends' handles.
//!
//! # Status
//!
//! Routing, cross-boundary lookup by name, and event tagging are
//! implemented and tested against two composed [`rivet_mock`] backends.
//! Rivet has not run this on a simulator that hosts two languages in one
//! process — that needs Questa, Xcelium, or VCS, none of which is available
//! to CI. `rivet-vpi` and `rivet-vhpi` do not implement `set_event_tag`
//! yet, so today a composite can only be built from backends that do; see
//! `docs/design/05-parity-plan.md` §8.
//!
//! [`rivet_mock`]: ../../rivet_mock/index.html

use crate::backend::{
    Action, Backend, BackendError, Capabilities, CbId, CbKind, Handle, ObjInfo, OwnedValue, Result, Value, WaveCmd,
};
use crate::runtime::Event;

/// Bits of a [`Handle`] reserved for the backend tag.
pub const TAG_BITS: u32 = 4;
/// Backends a composite can hold, the primary included.
pub const MAX_BACKENDS: usize = 1 << TAG_BITS;
const HANDLE_MASK: u32 = (1u32 << (32 - TAG_BITS)) - 1;
const CB_SHIFT: u32 = 56;
const CB_MASK: u64 = (1u64 << CB_SHIFT) - 1;

/// Tag a handle with the index of the backend that issued it.
///
/// Panics if the handle does not fit in `32 - TAG_BITS` bits; use
/// [`try_tag_handle`] where a simulator might hand out large indices.
pub fn tag_handle(tag: u8, h: Handle) -> Handle {
    try_tag_handle(tag, h).expect("handle does not fit alongside a composite backend tag")
}

/// [`tag_handle`], reporting overflow instead of panicking.
pub fn try_tag_handle(tag: u8, h: Handle) -> Result<Handle> {
    if h.0 > HANDLE_MASK {
        return Err(BackendError::Sim(format!(
            "handle {} does not fit in {} bits; a composite backend cannot tag it",
            h.0,
            32 - TAG_BITS
        )));
    }
    Ok(Handle(h.0 | ((tag as u32) << (32 - TAG_BITS))))
}

/// Split a tagged handle into `(tag, backend-local handle)`.
pub fn untag_handle(h: Handle) -> (u8, Handle) {
    ((h.0 >> (32 - TAG_BITS)) as u8, Handle(h.0 & HANDLE_MASK))
}

/// Tag a callback id with the index of the backend that issued it.
pub fn tag_cb(tag: u8, id: CbId) -> CbId {
    debug_assert!(id.0 <= CB_MASK, "callback id {} is too large to tag", id.0);
    CbId((id.0 & CB_MASK) | ((tag as u64) << CB_SHIFT))
}

/// Split a tagged callback id into `(tag, backend-local id)`.
pub fn untag_cb(id: CbId) -> (u8, CbId) {
    ((id.0 >> CB_SHIFT) as u8, CbId(id.0 & CB_MASK))
}

/// Apply a backend tag to the handle or callback id an event carries.
/// A secondary backend calls this on every event it dispatches.
pub fn tag_event(tag: u8, ev: Event) -> Event {
    match ev {
        Event::ValueChange(h) => Event::ValueChange(tag_handle(tag, h)),
        Event::Timer(id) => Event::Timer(tag_cb(tag, id)),
        other => other,
    }
}

/// Two or more backends presented to the runtime as one design.
///
/// The first backend is the *primary*: it owns time, the root of the
/// hierarchy, and every timing callback, because in a mixed-language
/// simulation there is only one kernel and one time wheel behind both
/// interfaces. Value lookups, reads, writes, and value-change callbacks go
/// to whichever backend owns the object.
pub struct CompositeBackend {
    backends: Vec<Box<dyn Backend>>,
    name: String,
    caps: Capabilities,
}

impl CompositeBackend {
    /// Compose `primary` with `secondaries`.
    ///
    /// Fails if there are too many backends, if they disagree about time
    /// precision (they are meant to be two views of one kernel), or if a
    /// secondary cannot tag the events it dispatches.
    pub fn new(primary: Box<dyn Backend>, secondaries: Vec<Box<dyn Backend>>) -> Result<CompositeBackend> {
        let mut backends = Vec::with_capacity(1 + secondaries.len());
        backends.push(primary);
        backends.extend(secondaries);
        if backends.len() > MAX_BACKENDS {
            return Err(BackendError::Unsupported(format!(
                "{} backends in one composite; the handle tag holds {MAX_BACKENDS}",
                backends.len()
            )));
        }
        let precision = backends[0].precision();
        for b in backends.iter().skip(1) {
            if b.precision() != precision {
                return Err(BackendError::Sim(format!(
                    "{} reports precision 1e{} but {} reports 1e{}; they cannot be two views of one kernel",
                    backends[0].name(),
                    precision,
                    b.name(),
                    b.precision()
                )));
            }
        }
        for (i, b) in backends.iter_mut().enumerate().skip(1) {
            b.set_event_tag(i as u8)?;
        }
        let name = backends.iter().map(|b| b.name().to_string()).collect::<Vec<_>>().join("+");
        // The safe reading of each flag: trust inertial writes only if every
        // backend does, remove fired callbacks if any backend needs it.
        let mut caps = backends[0].caps();
        for b in backends.iter().skip(1) {
            let c = b.caps();
            caps.trusts_inertial_writes &= c.trusts_inertial_writes;
            caps.remove_fired_callbacks |= c.remove_fired_callbacks;
            caps.four_state &= c.four_state;
            caps.supports_force &= c.supports_force;
        }
        Ok(CompositeBackend { backends, name, caps })
    }

    /// The backends, primary first.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    fn at(&mut self, tag: u8) -> Result<&mut dyn Backend> {
        match self.backends.get_mut(tag as usize) {
            Some(b) => Ok(&mut **b),
            None => Err(BackendError::Sim(format!("handle carries backend tag {tag}, which is not in this composite"))),
        }
    }

    /// Route a call that takes one handle, untagging on the way in.
    fn route<R>(&mut self, h: Handle, f: impl FnOnce(&mut dyn Backend, Handle) -> Result<R>) -> Result<R> {
        let (tag, local) = untag_handle(h);
        f(self.at(tag)?, local)
    }

    /// Look `path` up in `tag`'s backend, walking from its own root. Used
    /// to cross the language boundary: the VHDL half of the design is not
    /// reachable through VPI even by full path, but it is reachable through
    /// VHPI from the VHPI root.
    fn resolve_path(&mut self, tag: u8, path: &str) -> Result<Option<Handle>> {
        let b = self.at(tag)?;
        let root = b.root(None)?;
        let root_name = b.info(root).name.clone();
        let mut parts: Vec<&str> = path.split('.').collect();
        if parts.first() == Some(&root_name.as_str()) {
            parts.remove(0);
        }
        let mut h = root;
        for p in parts {
            match b.child_by_name(h, p)? {
                Some(c) => h = c,
                None => return Ok(None),
            }
        }
        Ok(Some(h))
    }
}

impl std::fmt::Debug for CompositeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeBackend").field("backends", &self.name).finish()
    }
}

impl Backend for CompositeBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> String {
        self.backends.iter().map(|b| format!("{} {}", b.name(), b.version())).collect::<Vec<_>>().join(" + ")
    }

    fn caps(&self) -> Capabilities {
        self.caps
    }

    fn precision(&self) -> i32 {
        self.backends[0].precision()
    }

    fn now(&self) -> u64 {
        self.backends[0].now()
    }

    fn root(&mut self, name: Option<&str>) -> Result<Handle> {
        match self.backends[0].root(name) {
            Ok(h) => try_tag_handle(0, h),
            Err(e) => {
                // A named top that the primary does not have may belong to
                // the other language.
                for tag in 1..self.backends.len() as u8 {
                    if let Ok(h) = self.at(tag)?.root(name) {
                        return try_tag_handle(tag, h);
                    }
                }
                Err(e)
            }
        }
    }

    fn child_by_name(&mut self, parent: Handle, name: &str) -> Result<Option<Handle>> {
        let (tag, local) = untag_handle(parent);
        if let Some(h) = self.at(tag)?.child_by_name(local, name)? {
            return try_tag_handle(tag, h).map(Some);
        }
        // Not in the owning backend: the child may be an instance of the
        // other language, which only the other interface can see.
        let path = format!("{}.{}", self.at(tag)?.info(local).path, name);
        for other in 0..self.backends.len() as u8 {
            if other == tag {
                continue;
            }
            if let Some(h) = self.resolve_path(other, &path)? {
                return try_tag_handle(other, h).map(Some);
            }
        }
        Ok(None)
    }

    fn child_by_index(&mut self, parent: Handle, index: i64) -> Result<Option<Handle>> {
        let (tag, local) = untag_handle(parent);
        match self.at(tag)?.child_by_index(local, index)? {
            Some(h) => try_tag_handle(tag, h).map(Some),
            None => Ok(None),
        }
    }

    fn children(&mut self, parent: Handle) -> Result<Vec<Handle>> {
        let (tag, local) = untag_handle(parent);
        let kids = self.at(tag)?.children(local)?;
        kids.into_iter().map(|h| try_tag_handle(tag, h)).collect()
    }

    fn info(&self, h: Handle) -> &ObjInfo {
        let (tag, local) = untag_handle(h);
        self.backends[tag as usize].info(local)
    }

    fn read(&mut self, h: Handle) -> Result<OwnedValue> {
        self.route(h, |b, h| b.read(h))
    }

    fn read_vec(&mut self, h: Handle, out: &mut crate::value::LogicVec) -> Result<()> {
        let (tag, local) = untag_handle(h);
        self.at(tag)?.read_vec(local, out)
    }

    fn write(&mut self, h: Handle, v: Value<'_>, action: Action) -> Result<()> {
        let (tag, local) = untag_handle(h);
        self.at(tag)?.write(local, v, action)
    }

    fn register(&mut self, kind: CbKind) -> Result<CbId> {
        match kind {
            // A value change is only observable through the interface that
            // owns the object.
            CbKind::ValueChange(h) => {
                let (tag, local) = untag_handle(h);
                let id = self.at(tag)?.register(CbKind::ValueChange(local))?;
                Ok(tag_cb(tag, id))
            }
            // One kernel, one time wheel: the primary schedules time.
            other => Ok(tag_cb(0, self.backends[0].register(other)?)),
        }
    }

    fn remove(&mut self, id: CbId) -> Result<()> {
        let (tag, local) = untag_cb(id);
        self.at(tag)?.remove(local)
    }

    fn enum_literals(&mut self, h: Handle) -> Option<Vec<String>> {
        let (tag, local) = untag_handle(h);
        self.backends.get_mut(tag as usize)?.enum_literals(local)
    }

    fn finish(&mut self) {
        for b in self.backends.iter_mut() {
            b.finish();
        }
    }

    fn argv(&self) -> Vec<String> {
        self.backends[0].argv()
    }

    fn waves(&mut self, cmd: WaveCmd) -> Result<()> {
        let mut last = None;
        let mut any = false;
        for b in self.backends.iter_mut() {
            match b.waves(cmd.clone()) {
                Ok(()) => any = true,
                Err(e) => last = Some(e),
            }
        }
        match (any, last) {
            (true, _) => Ok(()),
            (false, Some(e)) => Err(e),
            (false, None) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip() {
        for tag in 0..MAX_BACKENDS as u8 {
            for raw in [0u32, 1, 7, 1 << 20, HANDLE_MASK] {
                let (t, h) = untag_handle(tag_handle(tag, Handle(raw)));
                assert_eq!((t, h), (tag, Handle(raw)));
            }
            for raw in [0u64, 1, 1 << 40, CB_MASK] {
                let (t, id) = untag_cb(tag_cb(tag, CbId(raw)));
                assert_eq!((t, id), (tag, CbId(raw)));
            }
        }
        // Tag 0 is the identity, so the primary pays nothing.
        assert_eq!(tag_handle(0, Handle(12345)), Handle(12345));
        assert_eq!(tag_cb(0, CbId(12345)), CbId(12345));
    }

    #[test]
    fn a_handle_too_large_to_tag_is_reported() {
        assert!(try_tag_handle(1, Handle(HANDLE_MASK)).is_ok());
        let e = try_tag_handle(1, Handle(HANDLE_MASK + 1)).unwrap_err();
        assert!(e.to_string().contains("does not fit"), "{e}");
    }

    #[test]
    fn events_carry_the_tag() {
        assert_eq!(tag_event(2, Event::ValueChange(Handle(3))), Event::ValueChange(tag_handle(2, Handle(3))));
        assert_eq!(tag_event(2, Event::Timer(CbId(3))), Event::Timer(tag_cb(2, CbId(3))));
        assert_eq!(tag_event(2, Event::ReadWrite), Event::ReadWrite);
    }
}
