//! Where the server sends decoded controller state. Two adapters present it to
//! the PC (an emulated Steam Controller, an Xbox pad); [`MemorySink`] records it
//! for tests.

use dvc_common::controller::State;

pub trait Sink {
    /// Make the virtual controller look like `state`. Called for every accepted
    /// packet, and with a neutral state when the Deck goes quiet.
    fn apply(&mut self, state: &State);

    /// Whether the host can use the Deck's motion sensor data right now. When it
    /// can't, the Deck stops sending it (the server says so in its acks).
    fn wants_imu(&self) -> bool {
        true
    }
}

/// Records every state it is given.
pub struct MemorySink {
    pub applied: Vec<State>,
    /// What `wants_imu` answers; settable so tests can change the host's mind.
    pub imu_wanted: bool,
}

impl Default for MemorySink {
    fn default() -> Self {
        MemorySink {
            applied: Vec::new(),
            imu_wanted: true,
        }
    }
}

impl Sink for MemorySink {
    fn apply(&mut self, state: &State) {
        self.applied.push(*state);
    }

    fn wants_imu(&self) -> bool {
        self.imu_wanted
    }
}
