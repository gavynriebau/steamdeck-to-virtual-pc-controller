//! Parsing of the Steam Deck's raw HID state reports (`/dev/hidraw*`, 28de:1205).
//!
//! Reading hidraw works alongside Steam and is independent of any Steam Input
//! layout. Offsets follow the Deck report layout (as used by SDL) and were
//! checked against the real hardware.

use crate::controller::{Button, State, TRIGGER_MAX, axis};
use crate::lock;
use crate::status::{Report, Status};
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
pub fn find_nodes() -> Vec<PathBuf> {
    find_nodes_in(Path::new("/sys/class/hidraw"), Path::new("/dev"))
}

/// [`find_nodes`] with the sysfs and device directories given (for tests).
pub fn find_nodes_in(sys_hidraw: &Path, dev: &Path) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(sys_hidraw) else {
        return Vec::new();
    };
    let mut nodes: Vec<_> = dir
        .flatten()
        .filter(|e| {
            std::fs::read_to_string(e.path().join("device/uevent"))
                .is_ok_and(|u| u.contains("HID_ID=0003:000028DE:00001205"))
        })
        .map(|e| dev.join(e.file_name()))
        .collect();
    nodes.sort();
    nodes
}

/// Read hidraw reports from `reader` until it ends, passing each state report
/// to `on_state` (other reports are skipped). Stops early if `on_state`
/// returns false.
pub fn read_states(mut reader: impl Read, mut on_state: impl FnMut(State) -> bool) {
    let mut buf = [0u8; 128];
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        if let Some(state) = parse_report(&buf[..n])
            && !on_state(state)
        {
            break;
        }
    }
}

/// Start reading the Deck's controller in the background and send every state
/// to `tx`. The Deck exposes several hidraw nodes and only one emits state
/// reports, so all are read; nodes that disappear are picked up again when
/// they come back. Whether data is arriving goes to `report`; details to stderr.
pub fn spawn_reader(tx: Sender<State>, report: Report) {
    std::thread::spawn(move || {
        let got_data = Arc::new(AtomicBool::new(false));
        let active: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
        let mut warned = false;
        loop {
            for path in find_nodes() {
                if lock(&active).contains(&path) {
                    continue;
                }
                let Ok(file) = File::open(&path) else {
                    eprintln!("cannot open {}", path.display());
                    continue;
                };
                lock(&active).insert(path.clone());
                let (tx, got_data, active) = (tx.clone(), got_data.clone(), active.clone());
                let report = report.clone();
                std::thread::spawn(move || {
                    read_states(file, |state| {
                        if !got_data.swap(true, Ordering::Relaxed) {
                            eprintln!("controller data from {}", path.display());
                            report(Status::Controller(true));
                        }
                        tx.send(state).is_ok()
                    });
                    eprintln!("lost {}", path.display());
                    lock(&active).remove(&path);
                });
            }
            std::thread::sleep(Duration::from_secs(2));
            if !warned && !got_data.load(Ordering::Relaxed) {
                warned = true;
                eprintln!("no controller data yet (is the Deck controller available?)");
                report(Status::Controller(false));
            }
        }
    });
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

    /// A report captured from a real Deck lying flat with nothing pressed.
    const REAL_IDLE: &str = include_str!("fixtures/deck_idle_report.hex");

    fn hex(s: &str) -> Vec<u8> {
        let s = s.trim();
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn real_idle_report_parses_to_a_resting_deck() {
        let s = parse_report(&hex(REAL_IDLE)).unwrap();
        assert_eq!(s.button_bits(), 0, "nothing pressed");
        assert_eq!(s.hat, [0, 0]);
        assert_eq!(s.pads, [0; 4]);
        assert_eq!(s.pad_pressure, [0, 0]);
        assert_eq!(s.accel, [-373, -526, 16273], "about 1 g on Z: lying flat");
        assert_eq!(s.gyro, [-1, -2, 0]);
        assert_eq!(s.axes[axis::LEFT_TRIGGER], 0);
        assert_eq!(s.axes[axis::RIGHT_TRIGGER], 0);
        // Stick Y is flipped to down-positive: the Deck's raw Y is -1276 / -1508.
        assert_eq!(s.axes[axis::LEFT_Y], 1276);
        assert_eq!(s.axes[axis::RIGHT_Y], 1508);
    }

    #[test]
    fn real_report_with_view_and_menu_set_is_the_stop_chord() {
        let mut raw = hex(REAL_IDLE);
        // View (bit 12) + Menu (bit 14), as a real Deck sends them.
        raw[8..16].copy_from_slice(&0x5000u64.to_le_bytes());
        assert!(parse_report(&raw).unwrap().is_stop_chord());
    }

    /// A reader that returns one chunk per read, like hidraw returns one report.
    struct Chunks(std::vec::IntoIter<Vec<u8>>);

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.0.next() {
                Some(chunk) => {
                    buf[..chunk.len()].copy_from_slice(&chunk);
                    Ok(chunk.len())
                }
                None => Ok(0),
            }
        }
    }

    fn chunks(v: Vec<Vec<u8>>) -> Chunks {
        Chunks(v.into_iter())
    }

    #[test]
    fn read_states_yields_state_reports_and_skips_others() {
        let state = hex(REAL_IDLE);
        let other = vec![0x01, 0x00, 0x05, 0x01, 1, 2, 3]; // some other report type
        let mut got = Vec::new();
        read_states(chunks(vec![other, state.clone(), state]), |s| {
            got.push(s);
            true
        });
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn read_states_stops_when_the_callback_says_so() {
        let state = hex(REAL_IDLE);
        let mut count = 0;
        read_states(chunks(vec![state.clone(), state.clone(), state]), |_| {
            count += 1;
            false
        });
        assert_eq!(count, 1);
    }

    #[test]
    fn find_nodes_picks_only_the_deck_controller() {
        let root = std::env::temp_dir().join(format!("dvc-hidraw-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (node, id) in [
            ("hidraw0", "0003:000028DE:00001205"),
            ("hidraw1", "0003:000028DE:00001205"),
            ("hidraw2", "0018:00002808:00001015"), // the touchscreen
        ] {
            let dir = root.join("sys").join(node).join("device");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("uevent"), format!("DRIVER=x\nHID_ID={id}\n")).unwrap();
        }
        let nodes = find_nodes_in(&root.join("sys"), Path::new("/dev"));
        assert_eq!(
            nodes,
            [PathBuf::from("/dev/hidraw0"), PathBuf::from("/dev/hidraw1")]
        );
        assert!(find_nodes_in(&root.join("missing"), Path::new("/dev")).is_empty());
    }
}
