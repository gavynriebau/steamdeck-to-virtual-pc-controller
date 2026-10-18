//! Code shared by the Deck side and the PC side: what a controller's state is,
//! how it travels over the wire, and process start-up. Neither side may depend
//! on the other, only on this crate.

pub mod config;
pub mod controller;
pub mod log;
pub mod proto;
pub mod startup;
pub mod timing;
pub mod xbox_pad;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Lock a mutex, ignoring poisoning: the guarded state is plain data that is
/// always valid, and a panicked thread must not take the whole controller down.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
