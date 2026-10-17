//! Wire format: a fixed 24-byte UDP datagram describing the full gamepad state.
//!
//! The layout mirrors an Xbox 360 pad as exposed by the kernel (and by Steam
//! Input's virtual pad on the Deck): 13 buttons, 6 absolute axes, a 2-axis hat.

pub const MAGIC: [u8; 4] = *b"DVC1";
pub const PACKET_LEN: usize = 24;
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

/// evdev ABS codes for `axes`: X, Y, Z (left trigger), RX, RY, RZ (right trigger).
pub const AXES: [u16; 6] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05];
/// evdev ABS codes for `hat`: HAT0X, HAT0Y.
pub const HAT_AXES: [u16; 2] = [0x10, 0x11];

pub const STICK_MIN: i32 = -32768;
pub const STICK_MAX: i32 = 32767;
pub const TRIGGER_MAX: i32 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    pub buttons: u16,
    pub axes: [i16; 6],
    pub hat: [i8; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub seq: u32,
    pub state: State,
}

impl State {
    /// View (SELECT) + Menu (START) held together: the client's "stop streaming" chord.
    pub fn is_stop_chord(&self) -> bool {
        const CHORD: u16 = 1 << 6 | 1 << 7;
        self.buttons & CHORD == CHORD
    }
}

impl Packet {
    pub fn encode(&self) -> [u8; PACKET_LEN] {
        let mut b = [0u8; PACKET_LEN];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..8].copy_from_slice(&self.seq.to_le_bytes());
        b[8..10].copy_from_slice(&self.state.buttons.to_le_bytes());
        for (i, a) in self.state.axes.iter().enumerate() {
            b[10 + i * 2..12 + i * 2].copy_from_slice(&a.to_le_bytes());
        }
        b[22] = self.state.hat[0] as u8;
        b[23] = self.state.hat[1] as u8;
        b
    }

    pub fn decode(b: &[u8]) -> Option<Packet> {
        if b.len() != PACKET_LEN || b[0..4] != MAGIC {
            return None;
        }
        let mut axes = [0i16; 6];
        for (i, a) in axes.iter_mut().enumerate() {
            *a = i16::from_le_bytes([b[10 + i * 2], b[11 + i * 2]]);
        }
        Some(Packet {
            seq: u32::from_le_bytes(b[4..8].try_into().ok()?),
            state: State {
                buttons: u16::from_le_bytes([b[8], b[9]]),
                axes,
                hat: [b[22] as i8, b[23] as i8],
            },
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

    #[test]
    fn round_trip() {
        let p = Packet {
            seq: 0xdead_beef,
            state: State { buttons: 0b101_0000_0001, axes: [-32768, 32767, 0, -1, 255, 7], hat: [-1, 1] },
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
        assert_eq!(Packet::decode(b"DVC1"), None);
    }

    #[test]
    fn seq_wraps() {
        assert!(is_newer(1, 0));
        assert!(is_newer(0, u32::MAX));
        assert!(!is_newer(5, 5));
        assert!(!is_newer(0, 1));
    }
}
