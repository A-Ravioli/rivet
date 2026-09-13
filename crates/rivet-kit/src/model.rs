//! Reference models: an ordinary Rust struct the scoreboard consults.
//!
//! ```ignore
//! struct Adder;
//! impl Model<(u8, u8), u16> for Adder {
//!     fn step(&mut self, &(a, b): &(u8, u8)) -> Option<u16> { Some(a as u16 + b as u16) }
//! }
//! let mut sb = ModelScoreboard::new("adder", Adder);
//! sb.drive((3, 4));          // model predicts 7, scoreboard expects it
//! sb.observe(dut_sum);       // compared in order
//! sb.finish()?;
//! ```

use crate::scoreboard::Scoreboard;
use rivet_core::error::Result;
use std::fmt::Debug;

/// A model of the design: given an input transaction, produce the output
/// transaction the design should emit (or `None` if this input produces
/// none, such as a write to a FIFO).
pub trait Model<In, Out> {
    fn step(&mut self, input: &In) -> Option<Out>;

    /// Return to the post-reset state.
    fn reset(&mut self) {}
}

/// Any closure is a stateless model.
impl<In, Out, F: FnMut(&In) -> Option<Out>> Model<In, Out> for F {
    fn step(&mut self, input: &In) -> Option<Out> {
        self(input)
    }
}

/// A [`Scoreboard`] whose expected items come from a [`Model`].
pub struct ModelScoreboard<In, Out, M: Model<In, Out>> {
    model: M,
    sb: Scoreboard<Out>,
    driven: u64,
    _in: std::marker::PhantomData<In>,
}

impl<In, Out: PartialEq + Debug, M: Model<In, Out>> ModelScoreboard<In, Out, M> {
    pub fn new(name: &str, model: M) -> Self {
        ModelScoreboard { model, sb: Scoreboard::new(name), driven: 0, _in: std::marker::PhantomData }
    }

    /// Feed an input to the model; whatever it predicts becomes expected.
    pub fn drive(&mut self, input: In) {
        self.driven += 1;
        if let Some(out) = self.model.step(&input) {
            self.sb.expect(out);
        }
    }

    /// Record what the design produced.
    pub fn observe(&self, out: Out) {
        self.sb.observe(out);
    }

    pub fn model(&mut self) -> &mut M {
        &mut self.model
    }

    /// The underlying scoreboard (for sharing with a monitor task).
    pub fn scoreboard(&self) -> Scoreboard<Out> {
        self.sb.clone()
    }

    pub fn driven(&self) -> u64 {
        self.driven
    }

    pub fn reset(&mut self) {
        self.model.reset();
    }

    /// `Ok` if every prediction was observed in order and nothing else.
    pub fn finish(&self) -> Result<()> {
        self.sb.finish()
    }
}
