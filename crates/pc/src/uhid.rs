//! A virtual HID device created through `/dev/uhid`. This module knows the
//! kernel's UHID event layout and nothing about what the device is: callers
//! supply an [`Identity`] and a [`Handler`] that answers the host's reports.

use dvc_common::{info, lock};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

/// `sizeof(struct uhid_event)`.
const EVENT_LEN: usize = 4376;
const MAX_DATA: usize = 4096;

// Event types (`enum uhid_event_type`).
const UHID_DESTROY: u32 = 1;
const UHID_START: u32 = 2;
const UHID_GET_REPORT: u32 = 9;
const UHID_GET_REPORT_REPLY: u32 = 10;
const UHID_CREATE2: u32 = 11;
const UHID_INPUT2: u32 = 12;
const UHID_SET_REPORT: u32 = 13;
const UHID_SET_REPORT_REPLY: u32 = 14;

pub const BUS_USB: u16 = 3;

/// What the virtual device claims to be.
pub struct Identity {
    pub name: String,
    pub bus: u16,
    pub vendor: u32,
    pub product: u32,
    pub version: u32,
    pub report_descriptor: Vec<u8>,
}

/// Answers the traffic the host sends to the device (everything except input
/// reports, which flow the other way via [`UhidDevice::send_input`]).
pub trait Handler: Send + 'static {
    /// The host wrote a feature report (`report` starts with the report ID).
    fn set_report(&mut self, report: &[u8]);
    /// The host reads a feature report; the reply must start with the report ID.
    fn get_report(&mut self, report_number: u8) -> Vec<u8>;
}

fn get_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn get_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn put_u16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn new_event(kind: u32) -> Vec<u8> {
    let mut ev = vec![0u8; EVENT_LEN];
    put_u32(&mut ev, 0, kind);
    ev
}

fn create_event(identity: &Identity) -> Vec<u8> {
    let mut ev = new_event(UHID_CREATE2);
    let name = identity.name.as_bytes();
    let n = name.len().min(127);
    ev[4..4 + n].copy_from_slice(&name[..n]);
    let rd = &identity.report_descriptor;
    put_u16(&mut ev, 260, rd.len() as u16);
    put_u16(&mut ev, 262, identity.bus);
    put_u32(&mut ev, 264, identity.vendor);
    put_u32(&mut ev, 268, identity.product);
    put_u32(&mut ev, 272, identity.version);
    ev[280..280 + rd.len()].copy_from_slice(rd);
    ev
}

fn input_event(report: &[u8]) -> Vec<u8> {
    let mut ev = new_event(UHID_INPUT2);
    put_u16(&mut ev, 4, report.len() as u16);
    ev[6..6 + report.len()].copy_from_slice(report);
    ev
}

/// Answer one event the kernel sent: returns the reply event to write back, if
/// the event wants one. Pure apart from the handler, so it is tested without
/// `/dev/uhid`.
pub fn respond(event: &[u8], handler: &mut dyn Handler) -> Option<Vec<u8>> {
    match get_u32(event, 0) {
        UHID_SET_REPORT => {
            let size = (get_u16(event, 10) as usize).min(MAX_DATA);
            handler.set_report(&event[12..12 + size]);
            let mut reply = new_event(UHID_SET_REPORT_REPLY);
            put_u32(&mut reply, 4, get_u32(event, 4));
            Some(reply)
        }
        UHID_GET_REPORT => {
            let data = handler.get_report(event[8]);
            let size = data.len().min(MAX_DATA);
            let mut reply = new_event(UHID_GET_REPORT_REPLY);
            put_u32(&mut reply, 4, get_u32(event, 4));
            put_u16(&mut reply, 10, size as u16);
            reply[12..12 + size].copy_from_slice(&data[..size]);
            Some(reply)
        }
        _ => None,
    }
}

pub struct UhidDevice {
    file: Arc<Mutex<File>>,
}

impl UhidDevice {
    /// Create the device and start answering the host's requests on a
    /// background thread.
    pub fn create(identity: &Identity, mut handler: impl Handler) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uhid")?;
        file.write_all(&create_event(identity))?;

        let mut reader = file.try_clone()?;
        let file = Arc::new(Mutex::new(file));
        let writer = file.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; EVENT_LEN];
            while reader.read(&mut buf).is_ok() {
                if get_u32(&buf, 0) == UHID_START {
                    info!("uhid: device started");
                }
                if let Some(reply) = respond(&buf, &mut handler) {
                    let _ = lock(&writer).write_all(&reply);
                }
            }
        });
        Ok(UhidDevice { file })
    }

    /// Deliver an input report to the host.
    pub fn send_input(&self, report: &[u8]) -> io::Result<()> {
        lock(&self.file).write_all(&input_event(report))
    }
}

impl Drop for UhidDevice {
    fn drop(&mut self) {
        let _ = lock(&self.file).write_all(&new_event(UHID_DESTROY));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records what the host sent and answers with fixed bytes.
    #[derive(Default)]
    struct Recorder {
        set: Vec<Vec<u8>>,
        asked: Vec<u8>,
    }

    impl Handler for Recorder {
        fn set_report(&mut self, report: &[u8]) {
            self.set.push(report.to_vec());
        }
        fn get_report(&mut self, n: u8) -> Vec<u8> {
            self.asked.push(n);
            vec![n, 0xAA, 0xBB]
        }
    }

    fn set_report_event(id: u32, data: &[u8]) -> Vec<u8> {
        let mut ev = new_event(UHID_SET_REPORT);
        put_u32(&mut ev, 4, id);
        put_u16(&mut ev, 10, data.len() as u16);
        ev[12..12 + data.len()].copy_from_slice(data);
        ev
    }

    #[test]
    fn create_event_carries_the_identity_at_the_abi_offsets() {
        let identity = Identity {
            name: "Test Pad".into(),
            bus: BUS_USB,
            vendor: 0x28de,
            product: 0x1302,
            version: 0x0404,
            report_descriptor: vec![1, 2, 3],
        };
        let ev = create_event(&identity);
        assert_eq!(ev.len(), EVENT_LEN);
        assert_eq!(get_u32(&ev, 0), UHID_CREATE2);
        assert_eq!(&ev[4..12], b"Test Pad");
        assert_eq!(get_u16(&ev, 260), 3);
        assert_eq!(get_u16(&ev, 262), BUS_USB);
        assert_eq!(get_u32(&ev, 264), 0x28de);
        assert_eq!(get_u32(&ev, 268), 0x1302);
        assert_eq!(get_u32(&ev, 272), 0x0404);
        assert_eq!(&ev[280..283], &[1, 2, 3]);
    }

    #[test]
    fn input_event_wraps_the_report() {
        let ev = input_event(&[9, 8, 7]);
        assert_eq!(get_u32(&ev, 0), UHID_INPUT2);
        assert_eq!(get_u16(&ev, 4), 3);
        assert_eq!(&ev[6..9], &[9, 8, 7]);
    }

    #[test]
    fn set_report_reaches_the_handler_and_is_acknowledged_with_its_id() {
        let mut h = Recorder::default();
        let reply = respond(&set_report_event(42, &[1, 0x87, 3]), &mut h).unwrap();
        assert_eq!(h.set, [vec![1, 0x87, 3]]);
        assert_eq!(get_u32(&reply, 0), UHID_SET_REPORT_REPLY);
        assert_eq!(get_u32(&reply, 4), 42);
        assert_eq!(get_u16(&reply, 8), 0, "no error");
    }

    #[test]
    fn get_report_is_answered_with_the_handlers_bytes() {
        let mut h = Recorder::default();
        let mut ev = new_event(UHID_GET_REPORT);
        put_u32(&mut ev, 4, 7);
        ev[8] = 1;
        let reply = respond(&ev, &mut h).unwrap();
        assert_eq!(h.asked, [1]);
        assert_eq!(get_u32(&reply, 0), UHID_GET_REPORT_REPLY);
        assert_eq!(get_u32(&reply, 4), 7);
        assert_eq!(get_u16(&reply, 8), 0, "no error");
        assert_eq!(get_u16(&reply, 10), 3);
        assert_eq!(&reply[12..15], &[1, 0xAA, 0xBB]);
    }

    #[test]
    fn other_events_are_ignored() {
        let mut h = Recorder::default();
        assert!(respond(&new_event(UHID_START), &mut h).is_none());
        assert!(h.set.is_empty() && h.asked.is_empty());
    }

    #[test]
    fn oversized_sizes_from_the_kernel_do_not_panic() {
        let mut h = Recorder::default();
        let mut ev = set_report_event(1, &[]);
        put_u16(&mut ev, 10, u16::MAX);
        respond(&ev, &mut h).unwrap();
        assert_eq!(h.set[0].len(), MAX_DATA);
    }
}
