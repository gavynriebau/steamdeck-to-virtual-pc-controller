//! The evdev vocabulary of an Xbox 360 pad: which evdev codes stand for which
//! [`Button`] and axis. Used both to read Steam Input's virtual pad on the Deck
//! and to present an Xbox pad on the PC.

use crate::controller::Button;

/// Buttons with an evdev key code, in no particular order.
pub const BUTTONS: [(Button, u16); 13] = [
    (Button::A, 0x130),     // BTN_SOUTH
    (Button::B, 0x131),     // BTN_EAST
    (Button::X, 0x133),     // BTN_NORTH
    (Button::Y, 0x134),     // BTN_WEST
    (Button::L1, 0x136),    // BTN_TL
    (Button::R1, 0x137),    // BTN_TR
    (Button::View, 0x13a),  // BTN_SELECT
    (Button::Menu, 0x13b),  // BTN_START
    (Button::Steam, 0x13c), // BTN_MODE
    (Button::L3, 0x13d),    // BTN_THUMBL
    (Button::R3, 0x13e),    // BTN_THUMBR
    (Button::L4, 0x150),    // BTN_GEAR_DOWN
    (Button::R4, 0x151),    // BTN_GEAR_UP
];

/// How many of [`BUTTONS`] a real Xbox 360 pad has (the first N); the rest are extras.
pub const XBOX_BUTTON_COUNT: usize = 11;

/// evdev ABS codes for `State::axes`: X, Y, Z (left trigger), RX, RY, RZ (right trigger).
pub const AXES: [u16; 6] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05];
/// evdev ABS codes for `State::hat`: HAT0X, HAT0Y.
pub const HAT_AXES: [u16; 2] = [0x10, 0x11];

pub const STICK_MIN: i32 = -32768;
pub const STICK_MAX: i32 = 32767;

pub fn button_for_code(code: u16) -> Option<Button> {
    BUTTONS.iter().find(|&&(_, c)| c == code).map(|&(b, _)| b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for (button, code) in BUTTONS {
            assert_eq!(button_for_code(code), Some(button));
        }
        assert_eq!(button_for_code(0x999), None);
    }

    #[test]
    fn xbox_set_is_the_first_eleven() {
        assert_eq!(BUTTONS[XBOX_BUTTON_COUNT - 1].0, Button::R3);
        assert_eq!(BUTTONS[XBOX_BUTTON_COUNT].0, Button::L4);
    }
}
