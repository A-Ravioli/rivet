//! In-order scoreboard.

use rivet_core::error::{Error, Result};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::rc::Rc;

/// Compares observed items against expected items in order. Mismatches
/// are logged and counted; [`Scoreboard::finish`] turns them into an error.
#[derive(Clone)]
pub struct Scoreboard<T> {
    inner: Rc<RefCell<Inner<T>>>,
    name: String,
}

struct Inner<T> {
    expected: VecDeque<T>,
    matched: usize,
    errors: Vec<String>,
}

impl<T: PartialEq + Debug> Scoreboard<T> {
    pub fn new(name: &str) -> Scoreboard<T> {
        Scoreboard {
            inner: Rc::new(RefCell::new(Inner { expected: VecDeque::new(), matched: 0, errors: Vec::new() })),
            name: name.to_string(),
        }
    }

    /// Queue an expected item.
    pub fn expect(&self, item: T) {
        self.inner.borrow_mut().expected.push_back(item);
    }

    /// Record an observed item and compare it with the next expected one.
    pub fn observe(&self, item: T) {
        let mut g = self.inner.borrow_mut();
        match g.expected.pop_front() {
            Some(exp) if exp == item => g.matched += 1,
            Some(exp) => {
                let msg = format!(
                    "{}: mismatch at item {}: expected {exp:?}, got {item:?}",
                    self.name,
                    g.matched + g.errors.len()
                );
                log::error!("{msg}");
                g.errors.push(msg);
            }
            None => {
                let msg = format!("{}: unexpected item {item:?}", self.name);
                log::error!("{msg}");
                g.errors.push(msg);
            }
        }
    }

    pub fn matched(&self) -> usize {
        self.inner.borrow().matched
    }

    pub fn pending(&self) -> usize {
        self.inner.borrow().expected.len()
    }

    pub fn errors(&self) -> Vec<String> {
        self.inner.borrow().errors.clone()
    }

    /// `Ok` if everything matched and nothing is still expected.
    pub fn finish(&self) -> Result<()> {
        let g = self.inner.borrow();
        if !g.errors.is_empty() {
            return Err(Error::Msg(format!("{}: {} mismatch(es); first: {}", self.name, g.errors.len(), g.errors[0])));
        }
        if !g.expected.is_empty() {
            return Err(Error::Msg(format!(
                "{}: {} expected item(s) never observed; next: {:?}",
                self.name,
                g.expected.len(),
                g.expected[0]
            )));
        }
        log::info!("{}: {} item(s) matched", self.name, g.matched);
        Ok(())
    }
}
