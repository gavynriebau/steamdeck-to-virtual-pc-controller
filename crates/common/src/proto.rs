//! Wire format. The Deck sends a datagram with the full controller state on
//! every change and as a heartbeat; the server answers with a small ack now and
//! then so the Deck can tell it is being heard.
//!
//! The field order is written down once, in [`visit_fields`]; encoding,
//! decoding and the packet length all follow from it.

use crate::controller::State;
#[cfg(test)]
use crate::controller::{Imu, Pad};

pub const MAGIC: [u8; 4] = *b"DVC2";
pub const ACK_MAGIC: [u8; 4] = *b"DVA2";
pub const DEFAULT_PORT: u16 = 24800;
/// A buffer this big holds any datagram of this protocol.
pub const MAX_DATAGRAM: usize = 64;

/// A visitor over the numeric fields of a packet, in wire order. Each method
/// reads or writes the values it is handed, depending on the implementation.
trait Fields {
    fn u32(&mut self, v: &mut u32);
    fn i16(&mut self, v: &mut i16) {
        self.i16s(std::slice::from_mut(v));
    }
    fn u16(&mut self, v: &mut u16) {
        self.u16s(std::slice::from_mut(v));
    }
    fn i16s(&mut self, v: &mut [i16]);
    fn u16s(&mut self, v: &mut [u16]);
    fn i8s(&mut self, v: &mut [i8]);
}

/// THE wire layout, after the 4-byte magic. Add a field here and encode,
/// decode and [`packet_len`] all pick it up.
fn visit_fields(f: &mut impl Fields, seq: &mut u32, state: &mut State) {
    f.u32(seq);
    let mut buttons = state.button_bits();
    f.u32(&mut buttons);
    state.set_button_bits(buttons);
    f.i16s(&mut state.axes);
    f.i8s(&mut state.hat);
    // Pad positions first (left x, y, right x, y), then both pressures.
    f.i16(&mut state.left_pad.x);
    f.i16(&mut state.left_pad.y);
    f.i16(&mut state.right_pad.x);
    f.i16(&mut state.right_pad.y);
    f.u16(&mut state.left_pad.pressure);
    f.u16(&mut state.right_pad.pressure);
    f.i16s(&mut state.imu.accel);
    f.i16s(&mut state.imu.gyro);
}

struct Writer(Vec<u8>);

impl Fields for Writer {
    fn u32(&mut self, v: &mut u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i16s(&mut self, v: &mut [i16]) {
        v.iter()
            .for_each(|x| self.0.extend_from_slice(&x.to_le_bytes()));
    }
    fn u16s(&mut self, v: &mut [u16]) {
        v.iter()
            .for_each(|x| self.0.extend_from_slice(&x.to_le_bytes()));
    }
    fn i8s(&mut self, v: &mut [i8]) {
        v.iter()
            .for_each(|x| self.0.extend_from_slice(&x.to_le_bytes()));
    }
}

/// Reads values from `bytes`. The caller checks the length first, so running
/// out cannot happen; missing bytes would read as zero.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        match self.bytes.split_first_chunk::<N>() {
            Some((head, rest)) => {
                self.bytes = rest;
                *head
            }
            None => [0; N],
        }
    }
}

impl Fields for Reader<'_> {
    fn u32(&mut self, v: &mut u32) {
        *v = u32::from_le_bytes(self.take());
    }
    fn i16s(&mut self, v: &mut [i16]) {
        v.iter_mut()
            .for_each(|x| *x = i16::from_le_bytes(self.take()));
    }
    fn u16s(&mut self, v: &mut [u16]) {
        v.iter_mut()
            .for_each(|x| *x = u16::from_le_bytes(self.take()));
    }
    fn i8s(&mut self, v: &mut [i8]) {
        v.iter_mut()
            .for_each(|x| *x = i8::from_le_bytes(self.take()));
    }
}

/// Counts the bytes of the fields.
struct Sizer(usize);

impl Fields for Sizer {
    fn u32(&mut self, _: &mut u32) {
        self.0 += 4;
    }
    fn i16s(&mut self, v: &mut [i16]) {
        self.0 += 2 * v.len();
    }
    fn u16s(&mut self, v: &mut [u16]) {
        self.0 += 2 * v.len();
    }
    fn i8s(&mut self, v: &mut [i8]) {
        self.0 += v.len();
    }
}

/// Size in bytes of a state packet.
pub fn packet_len() -> usize {
    let mut sizer = Sizer(0);
    visit_fields(&mut sizer, &mut 0, &mut State::default());
    MAGIC.len() + sizer.0
}

/// Why a datagram was not accepted as a [`Packet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Not a packet of this protocol at all.
    NotOurs,
    /// This protocol, but another version (the ASCII version byte is given).
    Version(u8),
    /// Right protocol and version, wrong size.
    Length(usize),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::NotOurs => write!(f, "not a deck-virtual-controller packet"),
            DecodeError::Version(v) => write!(
                f,
                "protocol version {} (this build speaks {}); update the other side",
                *v as char, MAGIC[3] as char
            ),
            DecodeError::Length(n) => write!(f, "wrong packet size ({n} bytes)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub seq: u32,
    pub state: State,
}

impl Packet {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(MAGIC.to_vec());
        let (mut seq, mut state) = (self.seq, self.state);
        visit_fields(&mut w, &mut seq, &mut state);
        w.0
    }

    pub fn decode(b: &[u8]) -> Option<Packet> {
        Packet::try_decode(b).ok()
    }

    /// Like [`Packet::decode`], but says why a datagram was refused.
    pub fn try_decode(b: &[u8]) -> Result<Packet, DecodeError> {
        if b.len() < 4 || b[0..3] != MAGIC[0..3] {
            return Err(DecodeError::NotOurs);
        }
        if b[0..4] != MAGIC {
            return Err(DecodeError::Version(b[3]));
        }
        if b.len() != packet_len() {
            return Err(DecodeError::Length(b.len()));
        }
        let mut reader = Reader { bytes: &b[4..] };
        let (mut seq, mut state) = (0, State::default());
        visit_fields(&mut reader, &mut seq, &mut state);
        Ok(Packet { seq, state })
    }
}

/// The server's "I hear you": echoes the sequence number it last accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ack {
    pub seq: u32,
}

impl Ack {
    pub const LEN: usize = 8;

    pub fn encode(&self) -> [u8; Ack::LEN] {
        let mut b = [0u8; Ack::LEN];
        b[..4].copy_from_slice(&ACK_MAGIC);
        b[4..].copy_from_slice(&self.seq.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Ack> {
        if b.len() != Ack::LEN || b[..4] != ACK_MAGIC {
            return None;
        }
        Some(Ack {
            seq: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        })
    }
}

/// True if `seq` is newer than `last` (wrapping comparison).
pub fn is_newer(seq: u32, last: u32) -> bool {
    seq != last && seq.wrapping_sub(last) < u32::MAX / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gives every field a different non-zero value, so a field missing from
    /// the table (or swapped with another) cannot round-trip by accident.
    struct Filler(i32);

    impl Filler {
        fn next(&mut self) -> i32 {
            self.0 += 1;
            self.0
        }
    }

    impl Fields for Filler {
        fn u32(&mut self, v: &mut u32) {
            *v = 0x0101_0000 + self.next() as u32;
        }
        fn i16s(&mut self, v: &mut [i16]) {
            v.iter_mut().for_each(|x| *x = -(self.next() as i16) * 100);
        }
        fn u16s(&mut self, v: &mut [u16]) {
            v.iter_mut().for_each(|x| *x = 40_000 + self.next() as u16);
        }
        fn i8s(&mut self, v: &mut [i8]) {
            v.iter_mut().for_each(|x| *x = -(self.next() as i8));
        }
    }

    fn every_field_set() -> Packet {
        let (mut seq, mut state) = (0, State::default());
        visit_fields(&mut Filler(0), &mut seq, &mut state);
        Packet { seq, state }
    }

    #[test]
    fn every_field_round_trips() {
        let p = every_field_set();
        assert_eq!(Packet::decode(&p.encode()), Some(p));
    }

    #[test]
    fn the_packet_length_follows_the_table() {
        assert_eq!(packet_len(), 50);
        assert_eq!(every_field_set().encode().len(), packet_len());
        assert!(packet_len() <= MAX_DATAGRAM);
    }

    #[test]
    fn extreme_values_round_trip() {
        let mut state = State::from_button_bits(0x8005_0401);
        state.axes = [-32768, 32767, 0, -1, 255, 7];
        state.hat = [-1, 1];
        state.left_pad = Pad {
            x: -5,
            y: 6,
            pressure: 0,
        };
        state.right_pad = Pad {
            x: 32767,
            y: -32768,
            pressure: 65535,
        };
        let p = Packet {
            seq: 0xdead_beef,
            state,
        };
        assert_eq!(Packet::decode(&p.encode()), Some(p));
    }

    #[test]
    fn says_why_a_datagram_is_refused() {
        assert_eq!(Packet::try_decode(b"hello"), Err(DecodeError::NotOurs));
        assert_eq!(Packet::try_decode(b""), Err(DecodeError::NotOurs));
        let mut old = [0u8; 24];
        old[..4].copy_from_slice(b"DVC1");
        assert_eq!(Packet::try_decode(&old), Err(DecodeError::Version(b'1')));
        let mut short = [0u8; 10];
        short[..4].copy_from_slice(&MAGIC);
        assert_eq!(Packet::try_decode(&short), Err(DecodeError::Length(10)));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Packet::decode(&vec![0; packet_len()]), None);
        assert_eq!(Packet::decode(b"DVC2"), None);
    }

    #[test]
    fn ack_round_trips_and_is_not_a_packet() {
        let ack = Ack { seq: 0xabcd_1234 };
        assert_eq!(Ack::decode(&ack.encode()), Some(ack));
        assert_eq!(Packet::try_decode(&ack.encode()), Err(DecodeError::NotOurs));
        assert_eq!(Ack::decode(b"DVA2"), None);
        assert_eq!(Ack::decode(&[0; Ack::LEN]), None);
    }

    #[test]
    fn seq_wraps() {
        assert!(is_newer(1, 0));
        assert!(is_newer(0, u32::MAX));
        assert!(!is_newer(5, 5));
        assert!(!is_newer(0, 1));
    }

    /// The documented wire layout, spelled out byte by byte (built independently
    /// of the code, with Python's struct module): magic, seq, buttons, 6 axes,
    /// hat, pad x/y (left then right), pad pressure, accel, gyro, all little-endian.
    const GOLDEN: &str = "44564332443322115555050018fcd0070300fcff05000600ff016f0022ff4d0144fee803d0070a00ecff1e00d8ff3200c4ff";

    fn golden_packet() -> Packet {
        let mut state = State::from_button_bits(0x0005_5555);
        state.axes = [-1000, 2000, 3, -4, 5, 6];
        state.hat = [-1, 1];
        state.left_pad = Pad {
            x: 111,
            y: -222,
            pressure: 1000,
        };
        state.right_pad = Pad {
            x: 333,
            y: -444,
            pressure: 2000,
        };
        state.imu = Imu {
            accel: [10, -20, 30],
            gyro: [-40, 50, -60],
        };
        Packet {
            seq: 0x1122_3344,
            state,
        }
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn the_wire_bytes_match_the_documented_layout() {
        assert_eq!(golden_packet().encode(), unhex(GOLDEN));
    }

    #[test]
    fn the_documented_bytes_decode_to_the_expected_packet() {
        assert_eq!(Packet::decode(&unhex(GOLDEN)), Some(golden_packet()));
    }
}
