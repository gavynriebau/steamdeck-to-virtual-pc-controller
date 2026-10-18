//! The conversation with the host: the settings it writes, the feature-report
//! handshake, and the output reports (rumble, pulses) it sends. Holds the
//! device's state and turns what the host asks into events.

use super::identity::{ATTRIBUTES_REPLY, SERIAL};
use crate::uhid::Handler;
use dvc_common::lock;
use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

const FEATURE_LEN: usize = 64;

/// Setting register that selects which IMU data the controller sends. Steam
/// writes `0x18` (raw accelerometer + gyro) to turn it on and `0` to turn it off.
pub const SETTING_IMU_MODE: u8 = 0x30;
const IMU_ENABLED_BITS: u16 = 0x18;

/// Settings the host has written. Unwritten settings read as their defaults.
#[derive(Default, Debug)]
pub struct DeviceState {
    settings: BTreeMap<u8, u16>,
}

impl DeviceState {
    pub fn set(&mut self, register: u8, value: u16) {
        self.settings.insert(register, value);
    }

    /// Whether to send accelerometer and gyro data. On until the host turns it off.
    pub fn imu_enabled(&self) -> bool {
        self.settings
            .get(&SETTING_IMU_MODE)
            .is_none_or(|v| v & IMU_ENABLED_BITS != 0)
    }
}

/// Something the host asked of the controller that the Deck could act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    /// The host switched the IMU on or off.
    ImuEnabled(bool),
    /// Rumble motors: left and right speed.
    Rumble { left: u16, right: u16 },
    /// A haptic pulse on one trackpad (`side` 0 or 1).
    Pulse {
        side: u8,
        on: u16,
        off: u16,
        repeat: u16,
    },
}

/// The `(register, value)` pairs in a settings write
/// (`[1, 0x87, length, (register, value lo, value hi)...]`).
fn parse_settings_write(report: &[u8]) -> Vec<(u8, u16)> {
    if report.get(1) != Some(&0x87) {
        return Vec::new();
    }
    let len = (*report.get(2).unwrap_or(&0) as usize).min(report.len().saturating_sub(3));
    report[3..3 + len]
        .chunks_exact(3)
        .map(|t| (t[0], u16::from_le_bytes([t[1], t[2]])))
        .collect()
}

/// Decode an output report (`0x80` rumble, `0x81` pulse); other reports are ignored.
fn parse_output_report(report: &[u8]) -> Option<DeviceEvent> {
    let u16_at = |i: usize| Some(u16::from_le_bytes([*report.get(i)?, *report.get(i + 1)?]));
    match report.first()? {
        0x80 => Some(DeviceEvent::Rumble {
            left: u16_at(3)?,
            right: u16_at(6)?,
        }),
        0x81 => Some(DeviceEvent::Pulse {
            side: *report.get(1)?,
            on: u16_at(2)?,
            off: u16_at(4)?,
            repeat: u16_at(6)?,
        }),
        _ => None,
    }
}

/// The host's side of the conversation: feature-report handshake (the host
/// writes a command and then reads the answer), settings, and output reports.
pub(super) struct Handshake {
    last_command: Vec<u8>,
    device: Arc<Mutex<DeviceState>>,
    events: Option<Sender<DeviceEvent>>,
}

impl Handshake {
    fn emit(&self, event: DeviceEvent) {
        if let Some(tx) = &self.events {
            let _ = tx.send(event);
        }
    }
}

impl Handler for Handshake {
    fn set_report(&mut self, report: &[u8]) {
        self.last_command = report.to_vec();
        for (register, value) in parse_settings_write(report) {
            let mut device = lock(&self.device);
            let before = device.imu_enabled();
            device.set(register, value);
            let after = device.imu_enabled();
            drop(device);
            if before != after {
                self.emit(DeviceEvent::ImuEnabled(after));
            }
        }
    }

    fn get_report(&mut self, _report_number: u8) -> Vec<u8> {
        feature_reply(&self.last_command)
    }

    fn output_report(&mut self, report: &[u8]) {
        if let Some(event) = parse_output_report(report) {
            self.emit(event);
        }
    }
}

impl Handshake {
    pub(super) fn new(
        device: Arc<Mutex<DeviceState>>,
        events: Option<Sender<DeviceEvent>>,
    ) -> Self {
        Handshake {
            last_command: Vec::new(),
            device,
            events,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn handshake() -> Handshake {
        Handshake::new(Arc::new(Mutex::new(DeviceState::default())), None)
    }

    #[test]
    fn serial_query_gets_the_serial() {
        let mut h = handshake();
        h.set_report(&[1, 0xAE, 0x15, 0x01]);
        let r = h.get_report(1);
        assert_eq!(r.len(), FEATURE_LEN);
        assert_eq!(&r[..4], &[1, 0xAE, 0x14, 0x01]);
        assert_eq!(&r[4..4 + SERIAL.len()], SERIAL);
    }

    #[test]
    fn attributes_query_gets_the_captured_reply() {
        let mut h = handshake();
        h.set_report(&[1, 0x83, 0]);
        assert_eq!(
            &h.get_report(1)[..ATTRIBUTES_REPLY.len()],
            &ATTRIBUTES_REPLY
        );
    }

    #[test]
    fn unknown_commands_are_echoed_with_zero_length() {
        let mut h = handshake();
        h.set_report(&[1, 0xC1, 0x10]);
        assert_eq!(&h.get_report(1)[..3], &[1, 0xC1, 0]);
    }

    #[test]
    fn reads_answer_the_most_recent_command() {
        let mut h = handshake();
        h.set_report(&[1, 0x83, 0]);
        h.set_report(&[1, 0xAE, 0x15, 0x01]);
        assert_eq!(h.get_report(1)[1], 0xAE);
    }

    // Writes captured from the Steam client talking to the emulated controller.
    #[test]
    fn settings_writes_parse_into_register_value_pairs() {
        assert_eq!(
            parse_settings_write(&[1, 0x87, 3, 0x32, 0x84, 0x03]),
            [(0x32, 0x0384)]
        );
        assert_eq!(
            parse_settings_write(&[1, 0x87, 6, 0x30, 0x18, 0x00, 0x2e, 0x00, 0x00]),
            [(0x30, 0x18), (0x2e, 0)]
        );
        assert!(
            parse_settings_write(&[1, 0x83, 0]).is_empty(),
            "not a settings write"
        );
        assert_eq!(
            parse_settings_write(&[1, 0x87, 9, 0x30, 0x18]),
            [],
            "length beyond the report"
        );
    }

    #[test]
    fn imu_defaults_on_and_follows_the_host() {
        let mut d = DeviceState::default();
        assert!(d.imu_enabled());
        d.set(SETTING_IMU_MODE, 0);
        assert!(!d.imu_enabled());
        d.set(SETTING_IMU_MODE, 0x18);
        assert!(d.imu_enabled());
        d.set(0x2e, 0);
        assert!(d.imu_enabled(), "other registers do not affect the IMU");
    }

    #[test]
    fn handshake_reports_imu_toggles_once_per_change() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut h = Handshake::new(Arc::new(Mutex::new(DeviceState::default())), Some(tx));
        h.set_report(&[1, 0x87, 3, 0x30, 0x00, 0x00]); // off
        h.set_report(&[1, 0x87, 3, 0x30, 0x00, 0x00]); // still off: no event
        h.set_report(&[1, 0x87, 3, 0x30, 0x18, 0x00]); // on
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            events,
            [
                DeviceEvent::ImuEnabled(false),
                DeviceEvent::ImuEnabled(true)
            ]
        );
    }

    #[test]
    fn output_reports_become_haptic_events() {
        assert_eq!(
            parse_output_report(&[0x81, 1, 0x10, 0, 0x20, 0, 3, 0]),
            Some(DeviceEvent::Pulse {
                side: 1,
                on: 0x10,
                off: 0x20,
                repeat: 3
            })
        );
        assert_eq!(
            parse_output_report(&[0x80, 0, 0, 0x34, 0x12, 0, 0x78, 0x56, 0]),
            Some(DeviceEvent::Rumble {
                left: 0x1234,
                right: 0x5678
            })
        );
        assert_eq!(parse_output_report(&[0x82, 0]), None);
        assert_eq!(parse_output_report(&[0x81]), None, "truncated");
    }
}
