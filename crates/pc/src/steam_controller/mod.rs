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
use dvc_common::{error, lock};
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
    /// Create the emulated controller through the real `/dev/uhid`.
    pub fn create(name: &str) -> io::Result<Self> {
        Self::start(name, UhidDevice::create)
    }

    /// Create it over an already open UHID file (a fake kernel, in tests).
    pub fn create_on(file: std::fs::File, name: &str) -> io::Result<Self> {
        Self::start(name, |identity, handler| {
            UhidDevice::create_on(file, identity, handler)
        })
    }

    fn start(
        name: &str,
        open: impl FnOnce(&crate::uhid::Identity, Handshake) -> io::Result<UhidDevice>,
    ) -> io::Result<Self> {
        let device_state = Arc::new(Mutex::new(DeviceState::default()));
        let handshake = Handshake::new(device_state.clone());
        let device = open(&identity(name), handshake)?;
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
                error!("uhid write failed: {e}");
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uhid::testing::fake_kernel;
    use dvc_common::controller::{Button, Imu};
    use std::os::unix::net::UnixDatagram;

    // Event layout, as the kernel sees it.
    const EVENT_LEN: usize = 4376;
    const CREATE2: u32 = 11;
    const INPUT2: u32 = 12;
    const GET_REPORT: u32 = 9;
    const GET_REPORT_REPLY: u32 = 10;
    const SET_REPORT: u32 = 13;
    const SET_REPORT_REPLY: u32 = 14;

    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    fn next(kernel: &UnixDatagram) -> Vec<u8> {
        let mut buf = vec![0u8; EVENT_LEN];
        let n = kernel.recv(&mut buf).expect("the controller sent no event");
        buf.truncate(n);
        buf
    }

    /// What Steam does: write a feature command, then read the answer.
    fn steam_asks(kernel: &UnixDatagram, id: u32, command: &[u8]) -> Vec<u8> {
        let mut set = vec![0u8; EVENT_LEN];
        set[0..4].copy_from_slice(&SET_REPORT.to_le_bytes());
        set[4..8].copy_from_slice(&id.to_le_bytes());
        set[10..12].copy_from_slice(&(command.len() as u16).to_le_bytes());
        set[12..12 + command.len()].copy_from_slice(command);
        kernel.send(&set).unwrap();
        let ack = next(kernel);
        assert_eq!((u32_at(&ack, 0), u32_at(&ack, 4)), (SET_REPORT_REPLY, id));

        let mut get = vec![0u8; EVENT_LEN];
        get[0..4].copy_from_slice(&GET_REPORT.to_le_bytes());
        get[4..8].copy_from_slice(&(id + 1000).to_le_bytes());
        get[8] = 1;
        kernel.send(&get).unwrap();
        let reply = next(kernel);
        assert_eq!(
            (u32_at(&reply, 0), u32_at(&reply, 4)),
            (GET_REPORT_REPLY, id + 1000)
        );
        reply[12..12 + 64].to_vec()
    }

    /// Read events until an input report satisfies `want`; fail if none does in time.
    fn input_report_where(kernel: &UnixDatagram, want: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        for _ in 0..2000 {
            let ev = next(kernel);
            if u32_at(&ev, 0) == INPUT2 {
                let report = ev[6..6 + 54].to_vec();
                if want(&report) {
                    return report;
                }
            }
        }
        panic!("no input report matched");
    }

    fn start() -> (SteamSink, UnixDatagram) {
        let (file, kernel) = fake_kernel();
        let sink = SteamSink::create_on(file, "Test Controller").unwrap();
        let created = next(&kernel);
        assert_eq!(u32_at(&created, 0), CREATE2);
        assert_eq!(
            (u32_at(&created, 264), u32_at(&created, 268)),
            (0x28de, 0x1302)
        );
        (sink, kernel)
    }

    #[test]
    fn it_answers_steams_serial_query_like_the_real_controller() {
        let (_sink, kernel) = start();
        let reply = steam_asks(&kernel, 1, &[1, 0xAE, 0x15, 0x01]);
        assert_eq!(&reply[..4], &[1, 0xAE, 0x14, 0x01]);
        assert_eq!(&reply[4..4 + 13], b"DECKVC0000001");
    }

    #[test]
    fn states_applied_to_the_sink_arrive_as_numbered_input_reports() {
        let (mut sink, kernel) = start();
        let mut state = State::default().with(Button::A);
        state.axes[0] = 1234;
        sink.apply(&state);
        let report = input_report_where(&kernel, |r| r[2] & 1 == 1);
        assert_eq!(report[0], 0x42);
        assert_eq!(
            i16::from_le_bytes([report[10], report[11]]),
            1234,
            "left stick X"
        );
        let second = input_report_where(&kernel, |r| r[2] & 1 == 1);
        assert_ne!(
            report[1], second[1],
            "each report has the next sequence number"
        );
    }

    #[test]
    fn when_steam_switches_the_gyro_off_the_reports_carry_no_motion_data() {
        let (mut sink, kernel) = start();
        let mut state = State::default().with(Button::B);
        state.imu = Imu {
            accel: [111, 222, 333],
            gyro: [4, 5, 6],
        };
        sink.apply(&state);
        let with_motion = input_report_where(&kernel, |r| r[2] & 2 == 2);
        assert_eq!(i16::from_le_bytes([with_motion[34], with_motion[35]]), 111);
        assert!(sink.wants_imu());

        steam_asks(&kernel, 2, &[1, 0x87, 3, 0x30, 0x00, 0x00]); // IMU mode register: off
        assert!(
            !sink.wants_imu(),
            "the sink tells the server it does not need motion data"
        );
        let without = input_report_where(&kernel, |r| {
            r[2] & 2 == 2 && r[34..46].iter().all(|b| *b == 0)
        });
        assert_eq!(
            without[0], 0x42,
            "the button is still held, only the motion data is gone"
        );

        steam_asks(&kernel, 3, &[1, 0x87, 3, 0x30, 0x18, 0x00]); // and back on
        assert!(sink.wants_imu());
        let again = input_report_where(&kernel, |r| i16::from_le_bytes([r[34], r[35]]) == 111);
        assert_eq!(i16::from_le_bytes([again[34], again[35]]), 111);
    }
}
