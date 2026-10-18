//! Wire format: a fixed 50-byte UDP datagram carrying the full controller state.

use crate::controller::State;

pub const MAGIC: [u8; 4] = *b"DVC2";
pub const PACKET_LEN: usize = 50;
pub const DEFAULT_PORT: u16 = 24800;

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
        if b.len() != PACKET_LEN || b[0..4] != MAGIC {
            return None;
        }
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
