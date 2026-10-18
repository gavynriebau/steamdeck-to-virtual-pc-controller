//! The state of a game controller as this app sees it, independent of how it
//! is read (Deck hardware, evdev) or presented (Steam Controller, Xbox pad) and
//! of how it travels over the wire.

/// Every button the Deck can send. The discriminant is the button's bit in the
/// wire format, so the numbers must never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    A = 0,
    B = 1,
    X = 2,
    Y = 3,
    L1 = 4,
    R1 = 5,
    View = 6,
    Menu = 7,
    Steam = 8,
    L3 = 9,
    R3 = 10,
    L4 = 11,
    R4 = 12,
    L5 = 13,
    R5 = 14,
    LeftPadTouch = 15,
    RightPadTouch = 16,
    LeftPadClick = 17,
    RightPadClick = 18,
    QuickAccess = 19,
}

impl Button {
    const fn mask(self) -> u32 {
        1 << self as u32
    }
}

/// Indices into [`State::axes`].
pub mod axis {
    pub const LEFT_X: usize = 0;
    pub const LEFT_Y: usize = 1;
    pub const LEFT_TRIGGER: usize = 2;
    pub const RIGHT_X: usize = 3;
    pub const RIGHT_Y: usize = 4;
    pub const RIGHT_TRIGGER: usize = 5;
    /// The four stick axes (the others are triggers).
    pub const STICKS: [usize; 4] = [LEFT_X, LEFT_Y, RIGHT_X, RIGHT_Y];
}

/// Triggers span `0..=TRIGGER_MAX`; sticks span the full `i16` range.
pub const TRIGGER_MAX: i16 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    buttons: u32,
    /// Sticks and triggers, see [`axis`]. Stick Y is down-positive (evdev style).
    pub axes: [i16; 6],
    /// D-pad as -1/0/1 for X (right is +) and Y (down is +).
    pub hat: [i8; 2],
    /// Left X, left Y, right X, right Y (trackpads, up is positive).
    pub pads: [i16; 4],
    /// Left, right trackpad pressure.
    pub pad_pressure: [u16; 2],
    pub accel: [i16; 3],
    pub gyro: [i16; 3],
}

impl State {
    pub fn pressed(&self, button: Button) -> bool {
        self.buttons & button.mask() != 0
    }

    pub fn set(&mut self, button: Button, down: bool) {
        if down {
            self.buttons |= button.mask();
        } else {
            self.buttons &= !button.mask();
        }
    }

    /// This state with `button` held (handy for building states in tests).
    pub fn with(mut self, button: Button) -> Self {
        self.set(button, true);
        self
    }

    /// The raw button bits, as sent on the wire.
    pub fn button_bits(&self) -> u32 {
        self.buttons
    }

    pub fn set_button_bits(&mut self, bits: u32) {
        self.buttons = bits;
    }

    pub fn from_button_bits(bits: u32) -> Self {
        State {
            buttons: bits,
            ..State::default()
        }
    }

    /// View + Menu held together: the client's "stop streaming" chord.
    pub fn is_stop_chord(&self) -> bool {
        self.pressed(Button::View) && self.pressed(Button::Menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_bits_are_the_wire_format() {
        // Changing any of these breaks compatibility between Deck and PC builds.
        let expected = [
            (Button::A, 0),
            (Button::B, 1),
            (Button::X, 2),
            (Button::Y, 3),
            (Button::L1, 4),
            (Button::R1, 5),
            (Button::View, 6),
            (Button::Menu, 7),
            (Button::Steam, 8),
            (Button::L3, 9),
            (Button::R3, 10),
            (Button::L4, 11),
            (Button::R4, 12),
            (Button::L5, 13),
            (Button::R5, 14),
            (Button::LeftPadTouch, 15),
            (Button::RightPadTouch, 16),
            (Button::LeftPadClick, 17),
            (Button::RightPadClick, 18),
            (Button::QuickAccess, 19),
        ];
        for (button, bit) in expected {
            assert_eq!(
                State::default().with(button).button_bits(),
                1 << bit,
                "{button:?}"
            );
        }
    }

    #[test]
    fn set_and_pressed() {
        let mut s = State::default();
        s.set(Button::X, true);
        assert!(s.pressed(Button::X) && !s.pressed(Button::Y));
        s.set(Button::X, false);
        assert_eq!(s, State::default());
    }

    #[test]
    fn stop_chord_needs_both() {
        assert!(
            State::default()
                .with(Button::View)
                .with(Button::Menu)
                .is_stop_chord()
        );
        assert!(!State::default().with(Button::View).is_stop_chord());
        assert!(!State::default().with(Button::Menu).is_stop_chord());
    }
}
