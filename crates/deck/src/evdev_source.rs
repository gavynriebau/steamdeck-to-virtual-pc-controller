//! Reads a Linux gamepad (by default Steam Input's virtual Xbox pad on the
//! Deck) through evdev. The alternative to reading the Deck hardware directly.

use crate::source::{Candidate, DeviceReader, Discovery};
use dvc_common::controller::State;
use dvc_common::xbox_pad;
use dvc_common::{info, warn};
use evdev::EventSummary;
use evdev::raw_stream::{RawDevice, enumerate};
use std::collections::HashSet;
use std::path::PathBuf;

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

/// Reads one open evdev gamepad.
struct EvdevReader {
    device: RawDevice,
    state: State,
}

impl DeviceReader for EvdevReader {
    fn run(&mut self, on_state: &mut dyn FnMut(State) -> bool) {
        // An idle pad sends no events; report the resting state once so the
        // supervisor knows the device is alive.
        if !on_state(self.state) {
            return;
        }
        loop {
            let events = match self.device.fetch_events() {
                Ok(e) => e,
                Err(e) => {
                    warn!("device error: {e}");
                    return;
                }
            };
            let before = self.state;
            for ev in events {
                apply(&mut self.state, ev);
            }
            if self.state != before && !on_state(self.state) {
                return;
            }
        }
    }
}

/// Discovery of one evdev gamepad: `device` is a path, a device name, or empty
/// to auto-detect (Steam Input's virtual Xbox pad first).
pub struct EvdevDiscovery {
    pub device: String,
}

impl Discovery for EvdevDiscovery {
    fn present(&mut self, active: &HashSet<String>) -> Vec<Candidate> {
        // One pad is enough; searching opens every input device, so don't while reading one.
        if !active.is_empty() {
            return Vec::new();
        }
        let Some((path, device)) = find_device(&self.device) else {
            warn!("no gamepad device found, retrying");
            return Vec::new();
        };
        info!(
            "reading {} ({})",
            path.display(),
            device.name().unwrap_or("?")
        );
        vec![Candidate {
            id: path.display().to_string(),
            open: Box::new(move || {
                Ok(Box::new(EvdevReader {
                    device,
                    state: State::default(),
                }) as Box<dyn DeviceReader>)
            }),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvc_common::controller::{Button, axis};

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
