//! Fixtures: setup shared between tests.
//!
//! `#[rivet::fixture]` marks an async function that takes the dut and
//! returns a value a test can ask for by name. The generated test calls it
//! through [`acquire`], which accepts either the value or a `Result`, so a
//! fixture can use `?` internally without every test unwrapping twice.
//! Teardown is the value's `Drop`, which runs when the test future ends,
//! whether it passed, failed or was cancelled.

use crate::error::Result;

/// Accepts what a fixture returned: a value, or a `Result` of one.
pub trait Fixture<T> {
    fn into_result(self) -> Result<T>;
}

impl<T> Fixture<T> for Result<T> {
    fn into_result(self) -> Result<T> {
        self
    }
}

/// A fixture that cannot fail.
impl Fixture<crate::handle::Module> for crate::handle::Module {
    fn into_result(self) -> Result<crate::handle::Module> {
        Ok(self)
    }
}

impl Fixture<crate::handle::Signal> for crate::handle::Signal {
    fn into_result(self) -> Result<crate::handle::Signal> {
        Ok(self)
    }
}

impl Fixture<()> for () {
    fn into_result(self) -> Result<()> {
        Ok(())
    }
}

/// Await a fixture's future and unwrap what it produced.
pub async fn acquire<T, F, Fut>(fut: Fut) -> Result<T>
where
    Fut: std::future::Future<Output = F>,
    F: Fixture<T>,
{
    fut.await.into_result()
}
