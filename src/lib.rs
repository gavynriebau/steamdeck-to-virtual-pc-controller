//! Shared code for the Steam Deck virtual controller client (runs on the Deck)
//! and server (runs on the PC).

pub mod config;
pub mod controller;
pub mod deck_hid;
pub mod evdev_source;
pub mod link;
pub mod proto;
pub mod publisher;
pub mod serve;
pub mod session;
pub mod sink;
pub mod startup;
pub mod steam_controller;
pub mod uhid;
pub mod xbox_pad;
pub mod xbox_sink;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Lock a mutex, ignoring poisoning: the guarded state is plain data that is
/// always valid, and a panicked thread must not take the whole controller down.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
