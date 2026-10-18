//! The emulated 2026 Steam Controller ("Ibex", 28de:1302), copied from a real
//! controller so the kernel's `hid-steam` driver and the Steam client see the
//! same HID device a real wired controller presents.
//!
//! * `identity`: what it claims to be (USB IDs, report descriptor, captured replies);
//! * `device`: the host's conversation with it (settings, handshake);
//! * `report`: the `0x42` input report and its streamer;
//! * this module wires them to a UHID device and presents the result as a
//!   [`Sink`].

mod device;
mod identity;
mod report;

pub use device::{DeviceState, SETTING_IMU_MODE};
pub use identity::identity;
pub use report::state_report;

use crate::sink::Sink;
use crate::uhid::UhidDevice;
use device::Handshake;
use dvc_common::controller::State;
use dvc_common::lock;
use report::Streamer;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The emulated Steam Controller as a [`Sink`]: states handed to `apply` are
/// streamed to the host as input reports by a background thread, because Steam
/// drops a controller that goes quiet.
pub struct SteamSink {
    shared: Arc<Mutex<State>>,
    device: Arc<Mutex<DeviceState>>,
}

impl SteamSink {
    pub fn create(name: &str) -> io::Result<Self> {
        let device_state = Arc::new(Mutex::new(DeviceState::default()));
        let handshake = Handshake::new(device_state.clone());
        let device = UhidDevice::create(&identity(name), handshake)?;
        let shared = Arc::new(Mutex::new(State::default()));
        stream(
            device,
            shared.clone(),
            device_state.clone(),
            Duration::from_millis(4),
        );
        Ok(SteamSink {
            shared,
            device: device_state,
        })
    }
}

impl Sink for SteamSink {
    fn apply(&mut self, state: &State) {
        *lock(&self.shared) = *state;
    }

    fn wants_imu(&self) -> bool {
        lock(&self.device).imu_enabled()
    }
}

fn stream(
    device: UhidDevice,
    state: Arc<Mutex<State>>,
    device_state: Arc<Mutex<DeviceState>>,
    interval: Duration,
) {
    std::thread::spawn(move || {
        let mut streamer = Streamer::new(Instant::now());
        loop {
            std::thread::sleep(interval);
            let current = *lock(&state);
            let imu_enabled = lock(&device_state).imu_enabled();
            let report = streamer.tick(Instant::now(), &current, imu_enabled);
            if let Err(e) = device.send_input(&report) {
                eprintln!("uhid write failed: {e}");
                return;
            }
        }
    });
}
