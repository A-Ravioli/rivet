//! The user-facing error type.

use crate::backend::BackendError;
use crate::value::Unresolved;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    Backend(BackendError),
    Unresolved(Unresolved),
    Timeout(String),
    Cancelled,
    Panicked(String),
    /// The test does not apply to this simulator or design; it is recorded
    /// as skipped rather than failed. Raise it with [`crate::skip`].
    Skip(String),
    Msg(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Backend(e) => write!(f, "{e}"),
            Error::Unresolved(e) => write!(f, "{e}"),
            Error::Timeout(s) => write!(f, "timeout: {s}"),
            Error::Cancelled => write!(f, "task cancelled"),
            Error::Panicked(s) => write!(f, "task panicked: {s}"),
            Error::Skip(s) => write!(f, "skipped: {s}"),
            Error::Msg(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

impl From<BackendError> for Error {
    fn from(e: BackendError) -> Error {
        Error::Backend(e)
    }
}
impl From<Unresolved> for Error {
    fn from(e: Unresolved) -> Error {
        Error::Unresolved(e)
    }
}
impl From<String> for Error {
    fn from(s: String) -> Error {
        Error::Msg(s)
    }
}
impl From<&str> for Error {
    fn from(s: &str) -> Error {
        Error::Msg(s.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Return early with a formatted error.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => { return Err($crate::error::Error::Msg(format!($($arg)*))) };
}

/// Return early with a formatted error if the condition is false.
#[macro_export]
macro_rules! ensure {
    ($cond:expr $(,)?) => {
        if !$cond { return Err($crate::error::Error::Msg(format!("condition failed: {}", stringify!($cond)))) }
    };
    ($cond:expr, $($arg:tt)*) => {
        if !$cond { return Err($crate::error::Error::Msg(format!($($arg)*))) }
    };
}
