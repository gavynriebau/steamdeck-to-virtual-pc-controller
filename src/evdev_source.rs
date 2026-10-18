//! Reads a Linux gamepad (by default Steam Input's virtual Xbox pad on the
//! Deck) through evdev. The alternative to reading the Deck hardware directly.

use crate::controller::State;
use crate::status::{Report, Status};
use crate::xbox_pad;
use evdev::EventSummary;
use evdev::raw_stream::{RawDevice, enumerate};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Preference order when auto-detecting.
const PREFERRED: [&str; 2] = ["Microsoft X-Box 360 pad", "Valve Software Steam Controller"];

fn has_gamepad_buttons(d: &RawDevice) -> bool {
    d.supported_keys()
        .is_some_and(|k| k.contains(evdev::KeyCode(xbox_pad::BUTTONS[0].1)))
}

/// `want` is an evdev path, a device name, or empty to auto-detect.
fn find_device(want: &str) -> Option<(PathBuf, RawDevice)> {
    let mut found: Vec<_> = enumerate().collect();
    if !want.is_empty() {
        if want.starts_with('/') {
            return RawDevice::open(want).ok().map(|d| (want.into(), d));
        }
        return found.into_iter().find(|(_, d)| d.name() == Some(want));
    }
    found.retain(|(_, d)| has_gamepad_buttons(d));
    for pref in PREFERRED {
        if let Some(i) = found
            .iter()
            .position(|(_, d)| d.name().is_some_and(|n| n.starts_with(pref)))
        {
            return Some(found.swap_remove(i));
        }
    }
    None
}

/// Fold one evdev event into `state`.
fn apply(state: &mut State, ev: evdev::InputEvent) {
    match ev.destructure() {
        EventSummary::Key(_, code, v) => {
            if let Some(button) = xbox_pad::button_for_code(code.0) {
                state.set(button, v != 0);
            }
        }
        EventSummary::AbsoluteAxis(_, code, v) => {
            if let Some(i) = xbox_pad::AXES.iter().position(|&c| c == code.0) {
                state.axes[i] = v.clamp(xbox_pad::STICK_MIN, xbox_pad::STICK_MAX) as i16;
            } else if let Some(i) = xbox_pad::HAT_AXES.iter().position(|&c| c == code.0) {
                state.hat[i] = v.clamp(-1, 1) as i8;
            }
        }
        _ => {}
    }
}

/// Start reading in the background and send every changed state to `tx`.
/// When the device goes away the controller is released and the search resumes.
pub fn spawn(device: String, tx: Sender<State>, report: Report) {
    std::thread::spawn(move || {
        loop {
            let Some((path, mut dev)) = find_device(&device) else {
                eprintln!("no gamepad device found, retrying");
                report(Status::Controller(false));
                std::thread::sleep(Duration::from_secs(2));
                continue;
            };
            eprintln!("reading {} ({})", path.display(), dev.name().unwrap_or("?"));
            eprintln!("controller data from {}", path.display());
            report(Status::Controller(true));
            let mut state = State::default();
            loop {
                let events = match dev.fetch_events() {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!("device error: {e}");
                        break;
                    }
                };
                let before = state;
                for ev in events {
                    apply(&mut state, ev);
                }
                if state != before && tx.send(state).is_err() {
                    return;
                }
            }
            if tx.send(State::default()).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{Button, axis};

    const EV_KEY: u16 = 1;
    const EV_ABS: u16 = 3;

    fn ev(kind: u16, code: u16, value: i32) -> evdev::InputEvent {
        evdev::InputEvent::new(kind, code, value)
    }

    #[test]
    fn key_events_press_and_release_buttons() {
        let mut s = State::default();
        apply(&mut s, ev(EV_KEY, 0x130, 1));
        assert!(s.pressed(Button::A));
        apply(&mut s, ev(EV_KEY, 0x130, 0));
        assert_eq!(s, State::default());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let mut s = State::default();
        apply(&mut s, ev(EV_KEY, 0x999, 1));
        assert_eq!(s, State::default());
    }

    #[test]
    fn axes_are_clamped_and_hat_is_signed() {
        let mut s = State::default();
        apply(&mut s, ev(EV_ABS, 0x00, 100_000));
        apply(&mut s, ev(EV_ABS, 0x11, -1));
        apply(&mut s, ev(EV_ABS, 0x10, 5));
        assert_eq!(s.axes[axis::LEFT_X], i16::MAX);
        assert_eq!(s.hat, [1, -1]);
    }
}
