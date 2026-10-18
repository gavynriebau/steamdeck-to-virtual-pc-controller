//! Wire format: a fixed 50-byte UDP datagram carrying the full controller state.

use crate::controller::State;

pub const MAGIC: [u8; 4] = *b"DVC2";
pub const PACKET_LEN: usize = 50;
pub const DEFAULT_PORT: u16 = 24800;

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
    pub fn encode(&self) -> [u8; PACKET_LEN] {
        let st = &self.state;
        let mut w = Writer {
            b: [0u8; PACKET_LEN],
            at: 0,
        };
        w.bytes(&MAGIC);
        w.bytes(&self.seq.to_le_bytes());
        w.bytes(&st.button_bits().to_le_bytes());
        st.axes.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
        st.hat.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
        st.pads.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
        st.pad_pressure
            .iter()
            .for_each(|v| w.bytes(&v.to_le_bytes()));
        st.accel.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
        st.gyro.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
        w.b
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
        if b.len() != PACKET_LEN {
            return Err(DecodeError::Length(b.len()));
        }
        Packet::decode_body(b).ok_or(DecodeError::Length(b.len()))
    }

    fn decode_body(b: &[u8]) -> Option<Packet> {
        let mut r = Reader { b, at: 4 };
        let seq = u32::from_le_bytes(r.take()?);
        let mut st = State::from_button_bits(u32::from_le_bytes(r.take()?));
        for v in st.axes.iter_mut() {
            *v = i16::from_le_bytes(r.take()?);
        }
        for v in st.hat.iter_mut() {
            *v = i8::from_le_bytes(r.take()?);
        }
        for v in st.pads.iter_mut() {
            *v = i16::from_le_bytes(r.take()?);
        }
        for v in st.pad_pressure.iter_mut() {
            *v = u16::from_le_bytes(r.take()?);
        }
        for v in st.accel.iter_mut() {
            *v = i16::from_le_bytes(r.take()?);
        }
        for v in st.gyro.iter_mut() {
            *v = i16::from_le_bytes(r.take()?);
        }
        Some(Packet { seq, state: st })
    }
}

struct Writer {
    b: [u8; PACKET_LEN],
    at: usize,
}

impl Writer {
    fn bytes(&mut self, v: &[u8]) {
        self.b[self.at..self.at + v.len()].copy_from_slice(v);
        self.at += v.len();
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let v = self.b.get(self.at..self.at + N)?.try_into().ok()?;
        self.at += N;
        Some(v)
    }
}

/// True if `seq` is newer than `last` (wrapping comparison).
pub fn is_newer(seq: u32, last: u32) -> bool {
    seq != last && seq.wrapping_sub(last) < u32::MAX / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut state = State::from_button_bits(0x8005_0401);
        state.axes = [-32768, 32767, 0, -1, 255, 7];
        state.hat = [-1, 1];
        state.pads = [-5, 6, 32767, -32768];
        state.pad_pressure = [0, 65535];
        state.accel = [1, -2, 3];
        state.gyro = [-4, 5, -6];
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
        assert_eq!(Packet::decode(&[0; PACKET_LEN]), None);
        assert_eq!(Packet::decode(b"DVC2"), None);
    }

    #[test]
    fn seq_wraps() {
        assert!(is_newer(1, 0));
        assert!(is_newer(0, u32::MAX));
        assert!(!is_newer(5, 5));
        assert!(!is_newer(0, 1));
    }
}
