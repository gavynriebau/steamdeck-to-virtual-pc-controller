//! Wire format: a fixed 50-byte UDP datagram describing the full controller state.
//!
//! The layout mirrors an Xbox 360 pad as exposed by the kernel (and by Steam
//! Input's virtual pad on the Deck): 13 buttons, 6 absolute axes, a 2-axis hat;
//! plus what only the Steam Controller types use: extra buttons, two trackpads
//! and the IMU.

pub const MAGIC: [u8; 4] = *b"DVC2";
pub const PACKET_LEN: usize = 50;
pub const DEFAULT_PORT: u16 = 24800;

/// evdev key codes, in bitmask order (bit N of `buttons` is `BUTTONS[N]`).
pub const BUTTONS: [u16; 13] = [
    0x130, // BTN_SOUTH (A)
    0x131, // BTN_EAST (B)
    0x133, // BTN_NORTH (X)
    0x134, // BTN_WEST (Y)
    0x136, // BTN_TL
    0x137, // BTN_TR
    0x13a, // BTN_SELECT
    0x13b, // BTN_START
    0x13c, // BTN_MODE
    0x13d, // BTN_THUMBL
    0x13e, // BTN_THUMBR
    0x150, // BTN_GEAR_DOWN (L4)
    0x151, // BTN_GEAR_UP (R4)
];

/// Buttons 0..XBOX_BUTTONS are the standard Xbox 360 set; the rest are extras.
pub const XBOX_BUTTONS: usize = 11;

/// Extra button bits (no evdev code in the Xbox layout).
pub const BTN_L5: u32 = 13;
pub const BTN_R5: u32 = 14;
pub const BTN_LPAD_TOUCH: u32 = 15;
pub const BTN_RPAD_TOUCH: u32 = 16;
pub const BTN_LPAD_CLICK: u32 = 17;
pub const BTN_RPAD_CLICK: u32 = 18;
pub const BTN_QAM: u32 = 19;

/// evdev ABS codes for `axes`: X, Y, Z (left trigger), RX, RY, RZ (right trigger).
pub const AXES: [u16; 6] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05];
/// evdev ABS codes for `hat`: HAT0X, HAT0Y.
pub const HAT_AXES: [u16; 2] = [0x10, 0x11];

pub const STICK_MIN: i32 = -32768;
pub const STICK_MAX: i32 = 32767;
pub const TRIGGER_MAX: i32 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    pub buttons: u32,
    pub axes: [i16; 6],
    pub hat: [i8; 2],
    /// Left X, left Y, right X, right Y (trackpads, up is positive).
    pub pads: [i16; 4],
    /// Left, right trackpad pressure.
    pub pad_pressure: [u16; 2],
    pub accel: [i16; 3],
    pub gyro: [i16; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub seq: u32,
    pub state: State,
}

impl State {
    /// View (SELECT) + Menu (START) held together: the client's "stop streaming" chord.
    pub fn is_stop_chord(&self) -> bool {
        const CHORD: u32 = 1 << 6 | 1 << 7;
        self.buttons & CHORD == CHORD
    }
}

impl Packet {
    pub fn encode(&self) -> [u8; PACKET_LEN] {
        let st = &self.state;
        let mut w = Writer { b: [0u8; PACKET_LEN], at: 0 };
        w.bytes(&MAGIC);
        w.bytes(&self.seq.to_le_bytes());
        w.bytes(&st.buttons.to_le_bytes());
        st.axes.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
        st.hat.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
        st.pads.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
        st.pad_pressure.iter().for_each(|v| w.bytes(&v.to_le_bytes()));
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
        let buttons = u32::from_le_bytes(r.take()?);
        let mut st = State { buttons, ..State::default() };
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
        let p = Packet {
            seq: 0xdead_beef,
            state: State {
                buttons: 0x8005_0401,
                axes: [-32768, 32767, 0, -1, 255, 7],
                hat: [-1, 1],
                pads: [-5, 6, 32767, -32768],
                pad_pressure: [0, 65535],
                accel: [1, -2, 3],
                gyro: [-4, 5, -6],
            },
        };
        assert_eq!(Packet::decode(&p.encode()), Some(p));
    }

    #[test]
    fn stop_chord_needs_both() {
        assert!(State { buttons: 0b1100_0000, ..Default::default() }.is_stop_chord());
        assert!(!State { buttons: 0b0100_0000, ..Default::default() }.is_stop_chord());
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
