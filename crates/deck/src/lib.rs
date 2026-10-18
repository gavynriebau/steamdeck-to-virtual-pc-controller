//! The Steam Deck side: reads the Deck's controller and streams it to the PC.

pub mod config;
pub mod deck_hid;
pub mod evdev_source;
pub mod link;
pub mod publisher;
pub mod run;
pub mod source;
pub mod status;
