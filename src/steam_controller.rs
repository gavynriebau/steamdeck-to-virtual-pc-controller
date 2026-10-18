//! The emulated 2026 Steam Controller ("Ibex", 28de:1302): its identity, the
//! feature-report handshake and the `0x42` input report, all copied from a real
//! controller. The kernel's `hid-steam` driver and the Steam client then see the
//! same HID device a real wired controller presents.

use crate::controller::{Button, State, TRIGGER_MAX, axis};
use crate::lock;
use crate::sink::Sink;
use crate::uhid::{BUS_USB, Handler, Identity, UhidDevice};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const REPORT_LEN: usize = 54;
const FEATURE_LEN: usize = 64;

/// Report descriptor dumped from a real controller (hex, one line).
const REPORT_DESCRIPTOR_HEX: &str = include_str!("ibex_report_descriptor.hex");

fn report_descriptor() -> Vec<u8> {
    let hex = REPORT_DESCRIPTOR_HEX.trim();
    (0..hex.len() / 2)
        .filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// Captured reply to the `0x83` (get attributes) feature command.
const ATTRIBUTES_REPLY: [u8; 33] = [
    0x01, 0x83, 0x1e, 0x01, 0x02, 0x13, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x2e, 0xf9,
    0xd2, 0x68, 0x04, 0x23, 0xf3, 0xcf, 0x6a, 0x09, 0x48, 0x00, 0x00, 0x00, 0x0b, 0xa0, 0x0f, 0x00,
    0x00,
];

const SERIAL: &[u8] = b"DECKVC0000001";

/// What the Steam Controller claims to be.
pub fn identity(name: &str) -> Identity {
    Identity {
        name: name.to_string(),
        bus: BUS_USB,
        vendor: 0x28de,
        product: 0x1302,
        version: 0x0404,
        report_descriptor: report_descriptor(),
    }
}

/// The feature-report handshake: the host writes a command (`[1, command, len,
/// payload...]`) and then reads the answer.
#[derive(Default)]
struct Handshake {
    last_command: Vec<u8>,
}

impl Handler for Handshake {
    fn set_report(&mut self, report: &[u8]) {
        self.last_command = report.to_vec();
    }

    fn get_report(&mut self, _report_number: u8) -> Vec<u8> {
        feature_reply(&self.last_command)
    }
}

/// The emulated Steam Controller as a [`Sink`]: states handed to `apply` are
/// streamed to the host as input reports by a background thread, because Steam
/// drops a controller that goes quiet.
pub struct SteamSink {
    shared: Arc<Mutex<State>>,
}

impl SteamSink {
    pub fn create(name: &str) -> io::Result<Self> {
        let device = UhidDevice::create(&identity(name), Handshake::default())?;
        let shared = Arc::new(Mutex::new(State::default()));
        stream(device, shared.clone(), Duration::from_millis(4));
        Ok(SteamSink { shared })
    }
}

impl Sink for SteamSink {
    fn apply(&mut self, state: &State) {
        *lock(&self.shared) = *state;
    }
}

fn stream(device: UhidDevice, state: Arc<Mutex<State>>, interval: Duration) {
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut seq = 0u32;
        loop {
            std::thread::sleep(interval);
            seq = seq.wrapping_add(1);
            let st = *lock(&state);
            let ts = start.elapsed().as_micros() as u32;
            if let Err(e) = device.send_input(&state_report(seq, ts, &st)) {
                eprintln!("uhid write failed: {e}");
                return;
            }
        }
    });
}

/// Reply to a GET_REPORT, given the last command the host wrote
/// (`[report id 1, command, length, payload...]`).
fn feature_reply(cmd: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; FEATURE_LEN];
    match cmd.get(1) {
        // ID_GET_STRING_ATTRIBUTE: {1, 0xAE, len, ATTRIB_STR_UNIT_SERIAL, serial...}
        Some(0xAE) => {
            r[0..4].copy_from_slice(&[0x01, 0xAE, 0x14, 0x01]);
            r[4..4 + SERIAL.len()].copy_from_slice(SERIAL);
        }
        Some(0x83) => r[..ATTRIBUTES_REPLY.len()].copy_from_slice(&ATTRIBUTES_REPLY),
        Some(&c) => r[0..3].copy_from_slice(&[0x01, c, 0x00]),
        None => r[0] = 0x01,
    }
    r
}

/// The 54-byte `0x42` input report for a gamepad state.
pub fn state_report(seq: u32, ts_us: u32, st: &State) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[0] = 0x42;
    r[1] = seq as u8;

    let b = |button: Button| st.pressed(button);
    let trig = |v: i16| (v.clamp(0, TRIGGER_MAX) as i32 * 32767 / TRIGGER_MAX as i32) as i16;
    let (tl, tr) = (
        trig(st.axes[axis::LEFT_TRIGGER]),
        trig(st.axes[axis::RIGHT_TRIGGER]),
    );
    let set = |byte: &mut u8, bits: &[(bool, u8)]| {
        for &(on, bit) in bits {
            *byte |= (on as u8) << bit;
        }
    };
    // Bit positions were captured from a real controller.
    set(
        &mut r[2],
        &[
            (b(Button::A), 0),
            (b(Button::B), 1),
            (b(Button::X), 2),
            (b(Button::Y), 3),
            (b(Button::QuickAccess), 4),
            (b(Button::R3), 5),   // right stick click
            (b(Button::Menu), 6), // Start
            (b(Button::R4), 7),   // right top grip
        ],
    );
    set(
        &mut r[3],
        &[
            (b(Button::R5), 0), // right bottom grip
            (b(Button::R1), 1),
            (st.hat[1] > 0, 2),   // dpad down
            (st.hat[0] > 0, 3),   // dpad right
            (st.hat[0] < 0, 4),   // dpad left
            (st.hat[1] < 0, 5),   // dpad up
            (b(Button::View), 6), // Select
            (b(Button::L3), 7),   // left stick click
        ],
    );
    set(
        &mut r[4],
        &[
            (b(Button::Steam), 0),
            (b(Button::L4), 1), // left top grip
            (b(Button::L5), 2), // left bottom grip
            (b(Button::L1), 3),
            (b(Button::RightPadTouch), 5),
            (b(Button::RightPadClick), 6),
            (tr == 32767, 7), // right trigger full pull
        ],
    );
    r[5] = 0x30; // constant flag bits seen on the real device
    set(
        &mut r[5],
        &[
            (b(Button::LeftPadTouch), 1),
            (b(Button::LeftPadClick), 2),
            (tl == 32767, 3), // left trigger full pull
        ],
    );
    let put = |r: &mut [u8], off: usize, v: i16| r[off..off + 2].copy_from_slice(&v.to_le_bytes());
    put(&mut r, 6, tl);
    put(&mut r, 8, tr);
    // Sticks: the device reports "up is positive".
    put(&mut r, 10, st.axes[axis::LEFT_X]);
    put(&mut r, 12, st.axes[axis::LEFT_Y].saturating_neg());
    put(&mut r, 14, st.axes[axis::RIGHT_X]);
    put(&mut r, 16, st.axes[axis::RIGHT_Y].saturating_neg());
    // Trackpads (already up-positive) and their pressure.
    put(&mut r, 18, st.pads[0]);
    put(&mut r, 20, st.pads[1]);
    r[22..24].copy_from_slice(&st.pad_pressure[0].to_le_bytes());
    put(&mut r, 24, st.pads[2]);
    put(&mut r, 26, st.pads[3]);
    r[28..30].copy_from_slice(&st.pad_pressure[1].to_le_bytes());
    r[30..34].copy_from_slice(&ts_us.to_le_bytes());
    // IMU: the real device lays accel/gyro out as X, Z, Y. The Deck's sensor
    // uses the same units, so values are passed through.
    for (i, v) in st.accel.iter().enumerate() {
        put(&mut r, 34 + i * 2, *v);
    }
    for (i, v) in st.gyro.iter().enumerate() {
        put(&mut r, 40 + i * 2, *v);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_layout() {
        let mut st = State::default().with(Button::A).with(Button::Steam);
        st.axes = [100, -200, 255, 0, 0, 0];
        st.hat = [1, 0];
        st.pads = [7, -8, 0, 0];
        st.accel = [9, 0, 0];
        let r = state_report(7, 0, &st);
        assert_eq!(r[0], 0x42);
        assert_eq!(r[2] & 1, 1); // A
        assert_eq!(r[3] & 0x08, 0x08); // dpad right
        assert_eq!(r[4] & 1, 1); // Steam
        assert_eq!(i16::from_le_bytes([r[6], r[7]]), 32767); // left trigger
        assert_eq!(r[5] & 0x08, 0x08); // left trigger full pull
        assert_eq!(i16::from_le_bytes([r[12], r[13]]), 200);
        assert_eq!(i16::from_le_bytes([r[18], r[19]]), 7); // left pad X
        assert_eq!(i16::from_le_bytes([r[34], r[35]]), 9); // accel
    }

    /// Idle `0x42` report captured from a real controller: the fields we fill in
    /// must land at the same offsets with the same encoding.
    #[test]
    fn matches_captured_real_report() {
        const REAL: &str = "42d1000000300000000088fa54010cfe80fe000000000000000000000000c032210075037917cf3bb5ffefff0b007178390dbcfbf9d6";
        let real: Vec<u8> = (0..REAL.len() / 2)
            .map(|i| u8::from_str_radix(&REAL[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        // Sticks as sent over the wire are down-positive; the device is up-positive.
        let mut st = State::default();
        st.axes = [-1400, -340, 0, -500, 384, 0];
        st.accel = [885, 6009, 15311];
        st.gyro = [-75, -17, 11];
        let ours = state_report(0xd1, 2175680, &st);
        assert_eq!(real.len(), ours.len());
        assert_eq!(
            &ours[..30],
            &real[..30],
            "header, buttons, triggers, sticks, pads"
        );
        assert_eq!(&ours[30..46], &real[30..46], "timestamp, accel and gyro");
    }

    #[test]
    fn serial_query_gets_the_serial() {
        let mut h = Handshake::default();
        h.set_report(&[1, 0xAE, 0x15, 0x01]);
        let r = h.get_report(1);
        assert_eq!(r.len(), FEATURE_LEN);
        assert_eq!(&r[..4], &[1, 0xAE, 0x14, 0x01]);
        assert_eq!(&r[4..4 + SERIAL.len()], SERIAL);
    }

    #[test]
    fn attributes_query_gets_the_captured_reply() {
        let mut h = Handshake::default();
        h.set_report(&[1, 0x83, 0]);
        assert_eq!(
            &h.get_report(1)[..ATTRIBUTES_REPLY.len()],
            &ATTRIBUTES_REPLY
        );
    }

    #[test]
    fn unknown_commands_are_echoed_with_zero_length() {
        let mut h = Handshake::default();
        h.set_report(&[1, 0xC1, 0x10]);
        assert_eq!(&h.get_report(1)[..3], &[1, 0xC1, 0]);
    }

    #[test]
    fn reads_answer_the_most_recent_command() {
        let mut h = Handshake::default();
        h.set_report(&[1, 0x83, 0]);
        h.set_report(&[1, 0xAE, 0x15, 0x01]);
        assert_eq!(h.get_report(1)[1], 0xAE);
    }

    #[test]
    fn identity_is_the_wired_2026_controller() {
        let id = identity("x");
        assert_eq!((id.vendor, id.product, id.bus), (0x28de, 0x1302, BUS_USB));
        assert_eq!(id.report_descriptor.len(), 372);
    }
}
