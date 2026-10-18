//! The PC side: receives the Deck's controller state and presents it as an
//! emulated Steam Controller or an Xbox pad.

pub mod serve;
pub mod session;
pub mod sink;
pub mod steam_controller;
pub mod uhid;
pub mod xbox_sink;
