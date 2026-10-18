//! Presents the state as an Xbox 360 gamepad through uinput.

use crate::sink::Sink;
use dvc_common::controller::{self, Button, State};
use dvc_common::xbox_pad;
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, InputEvent, InputId, KeyCode, UinputAbsSetup,
};

const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;

/// How controller state maps onto the virtual device's evdev codes.
struct Layout {
    id: InputId,
    /// Button and its evdev key code.
    buttons: Vec<(Button, u16)>,
    /// evdev ABS code and range per `State::axes` entry.
    axes: [(u16, AbsInfo); 6],
    hat: [u16; 2],
}

impl Layout {
    /// Xbox 360 pad IDs so games/SDL apply their standard mapping.
    fn xbox() -> Layout {
        let stick = AbsInfo::new(0, xbox_pad::STICK_MIN, xbox_pad::STICK_MAX, 16, 128, 0);
        let trigger = AbsInfo::new(0, 0, controller::TRIGGER_MAX as i32, 0, 0, 0);
        let [lx, ly, lt, rx, ry, rt] = xbox_pad::AXES;
        Layout {
            id: InputId::new(BusType::BUS_USB, 0x045e, 0x028e, 0x0114),
            buttons: xbox_pad::BUTTONS[..xbox_pad::XBOX_BUTTON_COUNT].to_vec(),
            axes: [
                (lx, stick),
                (ly, stick),
                (lt, trigger),
                (rx, stick),
                (ry, stick),
                (rt, trigger),
            ],
            hat: xbox_pad::HAT_AXES,
        }
    }

    /// Events for everything that differs between `old` and `new`.
    fn diff(&self, old: &State, new: &State) -> Vec<InputEvent> {
        let mut ev = Vec::new();
        for &(button, code) in &self.buttons {
            let down = new.pressed(button);
            if old.pressed(button) != down {
                ev.push(InputEvent::new(EV_KEY, code, down as i32));
            }
        }
        for (i, &(code, _)) in self.axes.iter().enumerate() {
            if old.axes[i] != new.axes[i] {
                ev.push(InputEvent::new(EV_ABS, code, new.axes[i] as i32));
            }
        }
        for (i, &code) in self.hat.iter().enumerate() {
            if old.hat[i] != new.hat[i] {
                ev.push(InputEvent::new(EV_ABS, code, new.hat[i] as i32));
            }
        }
        ev
    }
}

pub struct XboxSink {
    device: VirtualDevice,
    layout: Layout,
    current: State,
}

impl XboxSink {
    pub fn create(name: &str) -> std::io::Result<Self> {
        let layout = Layout::xbox();
        let mut keys = AttributeSet::<KeyCode>::new();
        for &(_, code) in &layout.buttons {
            keys.insert(KeyCode(code));
        }
        let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
        let mut b = VirtualDevice::builder()?
            .name(name)
            .input_id(layout.id.clone())
            .with_keys(&keys)?;
        for &(code, info) in &layout.axes {
            b = b.with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode(code), info))?;
        }
        for &code in &layout.hat {
            b = b.with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode(code), hat))?;
        }
        Ok(XboxSink {
            device: b.build()?,
            layout,
            current: State::default(),
        })
    }
}

impl Sink for XboxSink {
    fn apply(&mut self, state: &State) {
        let events = self.layout.diff(&self.current, state);
        if events.is_empty() {
            return;
        }
        match self.device.emit(&events) {
            Ok(()) => self.current = *state,
            Err(e) => eprintln!("emit failed: {e}"),
        }
    }

    /// An Xbox pad has no motion sensors, so the Deck need not send any.
    fn wants_imu(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(ev: &[InputEvent]) -> Vec<(u16, u16, i32)> {
        ev.iter()
            .map(|e| (e.event_type().0, e.code(), e.value()))
            .collect()
    }

    #[test]
    fn pressing_a_emits_one_key_event() {
        let layout = Layout::xbox();
        let a = State::default().with(Button::A);
        assert_eq!(
            summary(&layout.diff(&State::default(), &a)),
            [(EV_KEY, 0x130, 1)]
        );
        assert_eq!(
            summary(&layout.diff(&a, &State::default())),
            [(EV_KEY, 0x130, 0)]
        );
    }

    #[test]
    fn unchanged_state_emits_nothing() {
        let layout = Layout::xbox();
        let s = State::default().with(Button::B);
        assert!(layout.diff(&s, &s).is_empty());
    }

    #[test]
    fn buttons_without_an_xbox_equivalent_are_dropped() {
        let layout = Layout::xbox();
        for b in [
            Button::L4,
            Button::L5,
            Button::QuickAccess,
            Button::LeftPadClick,
        ] {
            assert!(
                layout
                    .diff(&State::default(), &State::default().with(b))
                    .is_empty(),
                "{b:?}"
            );
        }
    }

    #[test]
    fn axes_and_hat_use_their_evdev_codes() {
        let layout = Layout::xbox();
        let mut s = State::default();
        s.axes[controller::axis::RIGHT_TRIGGER] = 200;
        s.hat = [1, -1];
        assert_eq!(
            summary(&layout.diff(&State::default(), &s)),
            [(EV_ABS, 0x05, 200), (EV_ABS, 0x10, 1), (EV_ABS, 0x11, -1)]
        );
    }
}
