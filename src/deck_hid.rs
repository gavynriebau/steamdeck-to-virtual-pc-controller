//! Parsing of the Steam Deck's raw HID state reports (`/dev/hidraw*`, 28de:1205).
//!
//! Reading hidraw works alongside Steam and is independent of any Steam Input
//! layout. Offsets follow the Deck report layout (as used by SDL) and were
//! checked against the real hardware.

use crate::proto::State;

pub const REPORT_LEN: usize = 64;
const HEADER: [u8; 4] = [0x01, 0x00, 0x09, 0x40];

/// (bit in the 64-bit button field, bit in `State::buttons`)
const BUTTON_MAP: [(u32, u16); 11] = [
    (7, 0),   // A
    (5, 1),   // B
    (6, 2),   // X
    (4, 3),   // Y
    (3, 4),   // L1
    (2, 5),   // R1
    (12, 6),  // View
    (14, 7),  // Menu
    (13, 8),  // Steam
    (22, 9),  // L3
    (26, 10), // R3
];
const BIT_L4: u32 = 41;
const BIT_R4: u32 = 42;
const DPAD_UP: u32 = 8;
const DPAD_RIGHT: u32 = 9;
const DPAD_LEFT: u32 = 10;
const DPAD_DOWN: u32 = 11;

fn i16_at(b: &[u8], off: usize) -> i16 {
    i16::from_le_bytes([b[off], b[off + 1]])
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

    let mut buttons = 0u16;
    for (src, dst) in BUTTON_MAP {
        if on(src) {
            buttons |= 1 << dst;
        }
    }
    if on(BIT_L4) {
        buttons |= 1 << 11;
    }
    if on(BIT_R4) {
        buttons |= 1 << 12;
    }

    let trigger = |off| (i16_at(b, off).max(0) >> 7) as i16; // 0..32767 -> 0..255
    // The Deck reports stick Y as up-positive; evdev expects up-negative.
    let axes = [
        i16_at(b, 48),
        neg(i16_at(b, 50)),
        trigger(44),
        i16_at(b, 52),
        neg(i16_at(b, 54)),
        trigger(46),
    ];
    let hat = [
        on(DPAD_RIGHT) as i8 - on(DPAD_LEFT) as i8,
        on(DPAD_DOWN) as i8 - on(DPAD_UP) as i8,
    ];
    Some(State { buttons, axes, hat })
}

/// hidraw nodes belonging to the Deck's built-in controller.
pub fn find_nodes() -> Vec<std::path::PathBuf> {
    let Ok(dir) = std::fs::read_dir("/sys/class/hidraw") else { return Vec::new() };
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
        let s = parse_report(&report(1 << 7 | 1 << 41 | 1 << 8 | 1 << 10, 1000, 32767, 32767)).unwrap();
        assert_eq!(s.buttons, 1 | 1 << 11);
        assert_eq!(s.hat, [-1, -1]);
        assert_eq!(s.axes[0], 1000);
        assert_eq!(s.axes[1], -32767);
        assert_eq!(s.axes[2], 255);
    }

    #[test]
    fn ignores_other_reports() {
        assert_eq!(parse_report(&[0u8; REPORT_LEN]), None);
        assert_eq!(parse_report(&[1, 0, 9, 0x40]), None);
    }

    #[test]
    fn y_min_does_not_overflow() {
        assert_eq!(parse_report(&report(0, 0, i16::MIN, 0)).unwrap().axes[1], i16::MAX);
    }
}
