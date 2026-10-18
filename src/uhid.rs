//! A virtual original Steam Controller (28de:1102, wired) created through
//! `/dev/uhid`. The kernel's `hid-steam` driver binds to it and exposes the
//! same evdev + hidraw nodes a real controller would, so Steam and SDL treat it
//! as a genuine Steam Controller.

use crate::proto::State;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

const EVENT_LEN: usize = 4376; // sizeof(struct uhid_event)

const UHID_DESTROY: u32 = 1;
const UHID_START: u32 = 2;
const UHID_CREATE2: u32 = 11;
const UHID_INPUT2: u32 = 12;
const UHID_GET_REPORT: u32 = 13;
const UHID_GET_REPORT_REPLY: u32 = 14;
const UHID_SET_REPORT: u32 = 15;
const UHID_SET_REPORT_REPLY: u32 = 16;

const BUS_USB: u16 = 3;
const REPORT_LEN: usize = 64;

/// One vendor-defined 64-byte input report and 64-byte feature report, no report IDs.
const REPORT_DESCRIPTOR: &[u8] = &[
    0x06, 0x00, 0xFF, // Usage Page (Vendor 0xFF00)
    0x09, 0x01, //       Usage (1)
    0xA1, 0x01, //       Collection (Application)
    0x09, 0x02, //         Usage (2)
    0x15, 0x00, //         Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x75, 0x08, //         Report Size (8)
    0x95, REPORT_LEN as u8, // Report Count (64)
    0x81, 0x02, //         Input (Data,Var,Abs)
    0x09, 0x03, //         Usage (3)
    0x95, REPORT_LEN as u8, // Report Count (64)
    0xB1, 0x02, //         Feature (Data,Var,Abs)
    0xC0, //             End Collection
];

const SERIAL: &[u8] = b"DECKVC0001";

pub struct SteamControllerDevice {
    file: Arc<Mutex<File>>,
}

fn put_u16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

impl SteamControllerDevice {
    pub fn create(name: &str) -> io::Result<Self> {
        let mut file = OpenOptions::new().read(true).write(true).open("/dev/uhid")?;
        let mut ev = vec![0u8; EVENT_LEN];
        put_u32(&mut ev, 0, UHID_CREATE2);
        let n = name.len().min(127);
        ev[4..4 + n].copy_from_slice(&name.as_bytes()[..n]);
        put_u16(&mut ev, 260, REPORT_DESCRIPTOR.len() as u16);
        put_u16(&mut ev, 262, BUS_USB);
        put_u32(&mut ev, 264, 0x28de);
        put_u32(&mut ev, 268, 0x1102);
        put_u32(&mut ev, 272, 0x0001);
        ev[280..280 + REPORT_DESCRIPTOR.len()].copy_from_slice(REPORT_DESCRIPTOR);
        file.write_all(&ev)?;

        // Answer the driver's feature-report traffic on a background thread.
        let mut reader = file.try_clone()?;
        let file = Arc::new(Mutex::new(file));
        let writer = file.clone();
        std::thread::spawn(move || {
            let mut last_cmd: Vec<u8> = Vec::new();
            let mut buf = vec![0u8; EVENT_LEN];
            while reader.read(&mut buf).is_ok() {
                match u32::from_le_bytes(buf[0..4].try_into().unwrap()) {
                    UHID_START => eprintln!("uhid: device started"),
                    UHID_SET_REPORT => {
                        let id = u32::from_le_bytes(buf[4..8].try_into().unwrap());
                        let size = u16::from_le_bytes(buf[10..12].try_into().unwrap()) as usize;
                        last_cmd = buf[12..12 + size.min(4096)].to_vec();
                        let mut r = vec![0u8; EVENT_LEN];
                        put_u32(&mut r, 0, UHID_SET_REPORT_REPLY);
                        put_u32(&mut r, 4, id);
                        let _ = writer.lock().unwrap().write_all(&r);
                    }
                    UHID_GET_REPORT => {
                        let id = u32::from_le_bytes(buf[4..8].try_into().unwrap());
                        // Report ID 0 is returned as a leading byte, which the driver strips.
                        let mut reply = vec![0u8];
                        reply.extend(feature_reply(&last_cmd));
                        let mut r = vec![0u8; EVENT_LEN];
                        put_u32(&mut r, 0, UHID_GET_REPORT_REPLY);
                        put_u32(&mut r, 4, id);
                        put_u16(&mut r, 8, 0);
                        put_u16(&mut r, 10, reply.len() as u16);
                        r[12..12 + reply.len()].copy_from_slice(&reply);
                        let _ = writer.lock().unwrap().write_all(&r);
                    }
                    _ => {}
                }
            }
        });
        Ok(SteamControllerDevice { file })
    }

    pub fn send_state(&self, seq: u32, st: &State) -> io::Result<()> {
        let report = state_report(seq, st);
        let mut ev = vec![0u8; EVENT_LEN];
        put_u32(&mut ev, 0, UHID_INPUT2);
        put_u16(&mut ev, 4, report.len() as u16);
        ev[6..6 + report.len()].copy_from_slice(&report);
        self.file.lock().unwrap().write_all(&ev)
    }
}

impl Drop for SteamControllerDevice {
    fn drop(&mut self) {
        let mut ev = vec![0u8; EVENT_LEN];
        put_u32(&mut ev, 0, UHID_DESTROY);
        let _ = self.file.lock().unwrap().write_all(&ev);
    }
}

/// Reply to a GET_REPORT, given the last command the driver wrote.
fn feature_reply(cmd: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; REPORT_LEN];
    match cmd.first() {
        // ID_GET_STRING_ATTRIBUTE: {0xAE, len, ATTRIB_STR_UNIT_SERIAL, serial...}
        Some(0xAE) => {
            r[0] = 0xAE;
            r[1] = (SERIAL.len() + 1) as u8;
            r[2] = 0x01;
            r[3..3 + SERIAL.len()].copy_from_slice(SERIAL);
        }
        // ID_GET_ATTRIBUTES_VALUES: no attributes.
        Some(0x83) => {
            r[0] = 0x83;
            r[1] = 5;
        }
        Some(&c) => {
            r[0] = c;
            r[1] = 1;
        }
        None => {}
    }
    r
}

/// The 64-byte `ID_CONTROLLER_STATE` input report for a gamepad state.
pub fn state_report(seq: u32, st: &State) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[0..4].copy_from_slice(&[0x01, 0x00, 0x01, 0x3c]);
    r[4..8].copy_from_slice(&seq.to_le_bytes());

    let b = |bit: usize| st.buttons >> bit & 1 != 0;
    let (trig_l, trig_r) = (st.axes[2].clamp(0, 255) as u8, st.axes[5].clamp(0, 255) as u8);
    let mut b8 = 0u8;
    for (on, bit) in [
        (trig_r == 255, 0), // right trigger click
        (trig_l == 255, 1), // left trigger click
        (b(5), 2),          // R1
        (b(4), 3),          // L1
        (b(3), 4),          // Y
        (b(1), 5),          // B
        (b(2), 6),          // X
        (b(0), 7),          // A
    ] {
        b8 |= (on as u8) << bit;
    }
    let mut b9 = 0u8;
    for (on, bit) in [
        (st.hat[1] < 0, 0), // up
        (st.hat[0] > 0, 1), // right
        (st.hat[0] < 0, 2), // left
        (st.hat[1] > 0, 3), // down
        (b(6), 4),          // View -> SELECT
        (b(8), 5),          // Steam -> MODE
        (b(7), 6),          // Menu -> START
        (b(11), 7),         // L4 -> left grip
    ] {
        b9 |= (on as u8) << bit;
    }
    let mut b10 = 0u8;
    for (on, bit) in [
        (b(12), 0), // R4 -> right grip
        (b(10), 2), // R3 -> right pad click
        (b(9), 6),  // L3 -> joystick click
    ] {
        b10 |= (on as u8) << bit;
    }
    r[8] = b8;
    r[9] = b9;
    r[10] = b10;
    r[11] = trig_l;
    r[12] = trig_r;
    // The kernel driver negates Y, so send "up is positive".
    r[16..18].copy_from_slice(&st.axes[0].to_le_bytes());
    r[18..20].copy_from_slice(&st.axes[1].saturating_neg().to_le_bytes());
    r[20..22].copy_from_slice(&st.axes[3].to_le_bytes());
    r[22..24].copy_from_slice(&st.axes[4].saturating_neg().to_le_bytes());
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_layout() {
        let st = State { buttons: 1 | 1 << 8, axes: [100, -200, 255, 0, 0, 0], hat: [1, 0] };
        let r = state_report(7, &st);
        assert_eq!(&r[0..4], &[1, 0, 1, 0x3c]);
        assert_eq!(r[8] & 0x80, 0x80); // A
        assert_eq!(r[9] & 0x22, 0x22); // dpad right + MODE
        assert_eq!(r[11], 255);
        assert_eq!(i16::from_le_bytes([r[18], r[19]]), 200);
    }
}
