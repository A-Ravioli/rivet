//! The simulator abstraction.
//!
//! A [`Backend`] wraps one procedural interface to one simulator (VPI, VHPI,
//! FLI, Verilator in-process, or the pure-Rust mock). It is responsible for
//! hierarchy discovery, value transport, and callback registration. It
//! reports events back to the runtime through [`crate::runtime::dispatch`].
//!
//! Backends are single-threaded and are only ever called from the
//! simulator's thread.

use crate::value::LogicVec;
use std::fmt;

/// A handle to a design object, valid for the life of the simulation.
/// Handles are indices into the backend's own table; they are `Copy` and
/// cheap to compare.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Handle(pub u32);

/// The kind of a design object, mirroring cocotb's `gpi_objtype`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ObjKind {
    /// A module, instance, interface, or scope.
    Module,
    /// A struct or record with named members.
    Struct,
    /// A generate array (pseudo-region).
    GenArray,
    Package,
    /// A single four-state bit.
    Logic,
    /// A packed vector of four-state bits (also packed structs).
    LogicVec,
    /// An unpacked array.
    Array,
    Integer,
    Real,
    String,
    Enum,
    Unknown,
}

impl ObjKind {
    /// Objects that hold a value readable as a `LogicVec`.
    pub fn is_logic_like(self) -> bool {
        matches!(self, ObjKind::Logic | ObjKind::LogicVec | ObjKind::Integer | ObjKind::Enum)
    }

    pub fn is_hierarchy(self) -> bool {
        matches!(self, ObjKind::Module | ObjKind::Struct | ObjKind::GenArray | ObjKind::Package)
    }
}

/// Cached metadata about a design object.
#[derive(Clone, Debug)]
pub struct ObjInfo {
    pub kind: ObjKind,
    /// Leaf name.
    pub name: String,
    /// Fully qualified path.
    pub path: String,
    /// Width in bits for value objects, element count for arrays.
    pub width: u32,
    pub is_const: bool,
    pub signed: bool,
    /// `(left, right)` declared range, if known.
    pub range: Option<(i64, i64)>,
    /// HDL type name as the simulator reports it.
    pub type_name: String,
}

/// How a write is applied.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Action {
    /// Inertial deposit: applied by the simulator when it next evaluates.
    /// The runtime may buffer these until the ReadWrite phase.
    Deposit,
    /// Immediate deposit.
    NoDelay,
    /// Override drivers until released.
    Force,
    /// Release a force; the value written is the current value.
    Release,
}

/// A value being written.
#[derive(Clone, Debug)]
pub enum Value<'a> {
    Vec(&'a LogicVec),
    Int(i64),
    Real(f64),
    Str(&'a str),
}

/// A value read back.
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedValue {
    Vec(LogicVec),
    Int(i64),
    Real(f64),
    Str(String),
}

/// A callback kind to register.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum CbKind {
    /// Fires on any value change of the object. Persistent until removed.
    ValueChange(Handle),
    /// Fires once after `steps` precision units.
    AfterDelay(u64),
    /// Fires once at the end of the current evaluation cycle.
    ReadWrite,
    /// Fires once at the end of the current time step.
    ReadOnly,
    /// Fires once at the start of the next time step.
    NextTimeStep,
}

/// Waveform dumping control (see [`crate::waves`]).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WaveCmd {
    On,
    Off,
    /// Switch to a new dump file (base name; the backend adds the extension).
    File(String),
}

/// Identifier of a registered callback.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct CbId(pub u64);

/// Per-backend behaviour flags.
#[derive(Copy, Clone, Debug)]
pub struct Capabilities {
    /// The simulator applies inertial deposits as the standard says, so the
    /// runtime can write through instead of buffering until ReadWrite.
    pub trusts_inertial_writes: bool,
    /// One-shot callbacks must be explicitly removed after they fire
    /// (Verilator treats them as recurring).
    pub remove_fired_callbacks: bool,
    /// Values carry X/Z.
    pub four_state: bool,
    /// Force/Release are supported.
    pub supports_force: bool,
}

/// Errors from the backend.
#[derive(Debug, Clone)]
pub enum BackendError {
    NotFound(String),
    WrongKind { path: String, expected: &'static str, actual: ObjKind },
    Unsupported(String),
    Sim(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::NotFound(p) => write!(f, "design object not found: {p}"),
            BackendError::WrongKind { path, expected, actual } => {
                write!(f, "{path} is a {actual:?}, expected {expected}")
            }
            BackendError::Unsupported(s) => write!(f, "unsupported: {s}"),
            BackendError::Sim(s) => write!(f, "simulator error: {s}"),
        }
    }
}

impl std::error::Error for BackendError {}

pub type Result<T> = std::result::Result<T, BackendError>;

/// The simulator interface.
pub trait Backend {
    fn name(&self) -> &str;
    fn version(&self) -> String;
    fn caps(&self) -> Capabilities;
    /// Time precision as an exponent of ten seconds (e.g. `-12`).
    fn precision(&self) -> i32;
    /// Current time in precision steps.
    fn now(&self) -> u64;

    /// The top-level instance. `name` filters when the design has several.
    fn root(&mut self, name: Option<&str>) -> Result<Handle>;
    fn child_by_name(&mut self, parent: Handle, name: &str) -> Result<Option<Handle>>;
    fn child_by_index(&mut self, parent: Handle, index: i64) -> Result<Option<Handle>>;
    fn children(&mut self, parent: Handle) -> Result<Vec<Handle>>;
    fn info(&self, h: Handle) -> &ObjInfo;

    fn read(&mut self, h: Handle) -> Result<OwnedValue>;
    /// Read a logic-like object straight into `out`, resizing it.
    fn read_vec(&mut self, h: Handle, out: &mut LogicVec) -> Result<()>;
    fn write(&mut self, h: Handle, v: Value<'_>, action: Action) -> Result<()>;

    fn register(&mut self, kind: CbKind) -> Result<CbId>;
    fn remove(&mut self, id: CbId) -> Result<()>;

    /// End the simulation.
    fn finish(&mut self);

    /// Simulator command line, if available.
    fn argv(&self) -> Vec<String> {
        Vec::new()
    }

    /// Control waveform dumping. Backends that cannot return `Unsupported`.
    fn waves(&mut self, cmd: WaveCmd) -> Result<()> {
        Err(BackendError::Unsupported(format!("waveform control ({cmd:?}) on {}", self.name())))
    }
}
