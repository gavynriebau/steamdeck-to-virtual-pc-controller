//! Parsing of the Steam Deck's raw HID state reports (`/dev/hidraw*`, 28de:1205).
//!
//! Reading hidraw works alongside Steam and is independent of any Steam Input
//! layout. Offsets follow the Deck report layout (as used by SDL) and were
//! checked against the real hardware.

use crate::controller::{Button, State, TRIGGER_MAX, axis};

pub const REPORT_LEN: usize = 64;
const HEADER: [u8; 4] = [0x01, 0x00, 0x09, 0x40];

/// (bit in the Deck's 64-bit button field, button)
const BUTTON_MAP: [(u32, Button); 20] = [
    (7, Button::A),
    (5, Button::B),
    (6, Button::X),
    (4, Button::Y),
    (3, Button::L1),
    (2, Button::R1),
    (12, Button::View),
    (14, Button::Menu),
    (13, Button::Steam),
    (22, Button::L3),
    (26, Button::R3),
    (41, Button::L4),
    (42, Button::R4),
    (15, Button::L5),
    (16, Button::R5),
    (19, Button::LeftPadTouch),
    (20, Button::RightPadTouch),
    (17, Button::LeftPadClick),
    (18, Button::RightPadClick),
    (50, Button::QuickAccess),
];
const DPAD_UP: u32 = 8;
const DPAD_RIGHT: u32 = 9;
const DPAD_LEFT: u32 = 10;
const DPAD_DOWN: u32 = 11;

fn i16_at(b: &[u8], off: usize) -> i16 {
    i16::from_le_bytes([b[off], b[off + 1]])
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

/// `-v` without overflowing at `i16::MIN`.
fn neg(v: i16) -> i16 {
    v.saturating_neg()
}

/// Returns the gamepad state in a state report, or `None` for any other report.
pub fn parse_report(b: &[u8]) -> Option<State> {
    if b.len() < REPORT_LEN || b[0..4] != HEADER {
        return None;
    }
    let bits = u64::from_le_bytes(b[8..16].try_into().ok()?);
    let on = |n: u32| bits >> n & 1 != 0;

    let mut state = State::default();
    for (bit, button) in BUTTON_MAP {
        state.set(button, on(bit));
    }

    // Triggers: 0..32767 on the Deck -> 0..=TRIGGER_MAX.
    let trigger = |off| i16_at(b, off).max(0) >> 7;
    debug_assert_eq!(i16::MAX >> 7, TRIGGER_MAX);
    // The Deck reports stick Y as up-positive; the state is down-positive.
    state.axes[axis::LEFT_X] = i16_at(b, 48);
    state.axes[axis::LEFT_Y] = neg(i16_at(b, 50));
    state.axes[axis::LEFT_TRIGGER] = trigger(44);
    state.axes[axis::RIGHT_X] = i16_at(b, 52);
    state.axes[axis::RIGHT_Y] = neg(i16_at(b, 54));
    state.axes[axis::RIGHT_TRIGGER] = trigger(46);
    state.hat = [
        on(DPAD_RIGHT) as i8 - on(DPAD_LEFT) as i8,
        on(DPAD_DOWN) as i8 - on(DPAD_UP) as i8,
    ];
    let s16 = |off| i16_at(b, off);
    state.pads = [s16(16), s16(18), s16(20), s16(22)];
    state.pad_pressure = [u16_at(b, 56), u16_at(b, 58)];
    state.accel = [s16(24), s16(26), s16(28)];
    state.gyro = [s16(30), s16(32), s16(34)];
    Some(state)
}

/// hidraw nodes belonging to the Deck's built-in controller.
pub fn find_nodes() -> Vec<std::path::PathBuf> {
    let Ok(dir) = std::fs::read_dir("/sys/class/hidraw") else {
        return Vec::new();
    };
    let mut nodes: Vec<_> = dir
        .flatten()
        .filter(|e| {
            std::fs::read_to_string(e.path().join("device/uevent"))
                .is_ok_and(|u| u.contains("HID_ID=0003:000028DE:00001205"))
        })
        .map(|e| std::path::Path::new("/dev").join(e.file_name()))
        .collect();
    nodes.sort();
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(bits: u64, lx: i16, ly: i16, tl: i16) -> [u8; REPORT_LEN] {
        let mut b = [0u8; REPORT_LEN];
        b[0..4].copy_from_slice(&HEADER);
        b[8..16].copy_from_slice(&bits.to_le_bytes());
        b[44..46].copy_from_slice(&tl.to_le_bytes());
        b[48..50].copy_from_slice(&lx.to_le_bytes());
        b[50..52].copy_from_slice(&ly.to_le_bytes());
        b
    }

    #[test]
    fn buttons_hat_axes() {
        let s = parse_report(&report(
            1 << 7 | 1 << 41 | 1 << 8 | 1 << 10,
            1000,
            32767,
            32767,
        ))
        .unwrap();
        assert!(s.pressed(Button::A) && s.pressed(Button::L4));
        assert!(!s.pressed(Button::B));
        assert_eq!(s.pads, [0; 4]);
        assert_eq!(s.hat, [-1, -1]);
        assert_eq!(s.axes[0], 1000);
        assert_eq!(s.axes[1], -32767);
        assert_eq!(s.axes[2], 255);
    }

    /// View + Menu as reported by a real Deck (`0x5000`, plus the extra bits
    /// 46/47 it sets while a button is held) must trigger the stop chord.
    #[test]
    fn view_plus_menu_is_the_stop_chord() {
        let both = parse_report(&report(0x5000 | 3 << 46, 0, 0, 0)).unwrap();
        assert!(both.is_stop_chord());
        assert!(
            !parse_report(&report(0x1000, 0, 0, 0))
                .unwrap()
                .is_stop_chord()
        );
        assert!(
            !parse_report(&report(0x4000, 0, 0, 0))
                .unwrap()
                .is_stop_chord()
        );
    }

    #[test]
    fn ignores_other_reports() {
        assert_eq!(parse_report(&[0u8; REPORT_LEN]), None);
        assert_eq!(parse_report(&[1, 0, 9, 0x40]), None);
    }

    #[test]
    fn y_min_does_not_overflow() {
        assert_eq!(
            parse_report(&report(0, 0, i16::MIN, 0)).unwrap().axes[1],
            i16::MAX
        );
    }
}
