//! Where the server sends decoded controller state. Two adapters present it to
//! the PC (an emulated Steam Controller, an Xbox pad); [`MemorySink`] records it
//! for tests.

use crate::controller::State;

pub trait Sink {
    /// Make the virtual controller look like `state`. Called for every accepted
    /// packet, and with a neutral state when the Deck goes quiet.
    fn apply(&mut self, state: &State);
}

/// Records every state it is given.
#[derive(Default)]
pub struct MemorySink {
    pub applied: Vec<State>,
}

impl Sink for MemorySink {
    fn apply(&mut self, state: &State) {
        self.applied.push(*state);
    }
}
