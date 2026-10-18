//! The `0x42` input report, as the real controller lays it out, and the
//! streamer that numbers and timestamps a series of them. Pure: no devices, no
//! threads, no clock of its own.

use dvc_common::controller::{Button, Imu, State, TRIGGER_MAX, axis};
use std::time::Instant;

pub(super) const REPORT_LEN: usize = 54;

/// The state as the host should see it: IMU data is zeroed while the host has it off.
pub(super) fn gate_imu(state: &State, imu_enabled: bool) -> State {
    let mut gated = *state;
    if !imu_enabled {
        gated.imu = Imu::default();
    }
    gated
}

/// Turns the current state into the stream of numbered, timestamped input
/// reports the host expects. One call per report; time is passed in.
pub(super) struct Streamer {
    start: Instant,
    seq: u32,
}

impl Streamer {
    pub(super) fn new(start: Instant) -> Self {
        Streamer { start, seq: 0 }
    }

    pub(super) fn tick(
        &mut self,
        now: Instant,
        state: &State,
        imu_enabled: bool,
    ) -> [u8; REPORT_LEN] {
        self.seq = self.seq.wrapping_add(1);
        let micros = now.duration_since(self.start).as_micros() as u32;
        state_report(self.seq, micros, &gate_imu(state, imu_enabled))
    }
}

/// The 54-byte `0x42` input report for a gamepad state.
pub fn state_report(seq: u32, ts_us: u32, st: &State) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[0] = 0x42;
    r[1] = seq as u8;

    let b = |button: Button| st.pressed(button);
    let trig = |v: i16| (v.clamp(0, TRIGGER_MAX) as i32 * 32767 / TRIGGER_MAX as i32) as i16;
    let (tl, tr) = (
        trig(st.axes[axis::LEFT_TRIGGER]),
        trig(st.axes[axis::RIGHT_TRIGGER]),
    );
    let set = |byte: &mut u8, bits: &[(bool, u8)]| {
        for &(on, bit) in bits {
            *byte |= (on as u8) << bit;
        }
    };
    // Bit positions were captured from a real controller.
    set(
        &mut r[2],
        &[
            (b(Button::A), 0),
            (b(Button::B), 1),
            (b(Button::X), 2),
            (b(Button::Y), 3),
            (b(Button::QuickAccess), 4),
            (b(Button::R3), 5),   // right stick click
            (b(Button::Menu), 6), // Start
            (b(Button::R4), 7),   // right top grip
        ],
    );
    set(
        &mut r[3],
        &[
            (b(Button::R5), 0), // right bottom grip
            (b(Button::R1), 1),
            (st.hat[1] > 0, 2),   // dpad down
            (st.hat[0] > 0, 3),   // dpad right
            (st.hat[0] < 0, 4),   // dpad left
            (st.hat[1] < 0, 5),   // dpad up
            (b(Button::View), 6), // Select
            (b(Button::L3), 7),   // left stick click
        ],
    );
    set(
        &mut r[4],
        &[
            (b(Button::Steam), 0),
            (b(Button::L4), 1), // left top grip
            (b(Button::L5), 2), // left bottom grip
            (b(Button::L1), 3),
            (b(Button::RightPadTouch), 5),
            (b(Button::RightPadClick), 6),
            (tr == 32767, 7), // right trigger full pull
        ],
    );
    r[5] = 0x30; // constant flag bits seen on the real device
    set(
        &mut r[5],
        &[
            (b(Button::LeftPadTouch), 1),
            (b(Button::LeftPadClick), 2),
            (tl == 32767, 3), // left trigger full pull
        ],
    );
    let put = |r: &mut [u8], off: usize, v: i16| r[off..off + 2].copy_from_slice(&v.to_le_bytes());
    put(&mut r, 6, tl);
    put(&mut r, 8, tr);
    // Sticks: the device reports "up is positive".
    put(&mut r, 10, st.axes[axis::LEFT_X]);
    put(&mut r, 12, st.axes[axis::LEFT_Y].saturating_neg());
    put(&mut r, 14, st.axes[axis::RIGHT_X]);
    put(&mut r, 16, st.axes[axis::RIGHT_Y].saturating_neg());
    // Trackpads (already up-positive) and their pressure.
    put(&mut r, 18, st.left_pad.x);
    put(&mut r, 20, st.left_pad.y);
    r[22..24].copy_from_slice(&st.left_pad.pressure.to_le_bytes());
    put(&mut r, 24, st.right_pad.x);
    put(&mut r, 26, st.right_pad.y);
    r[28..30].copy_from_slice(&st.right_pad.pressure.to_le_bytes());
    r[30..34].copy_from_slice(&ts_us.to_le_bytes());
    // IMU: the real device lays accel/gyro out as X, Z, Y. The Deck's sensor
    // uses the same units, so values are passed through.
    for (i, v) in st.imu.accel.iter().enumerate() {
        put(&mut r, 34 + i * 2, *v);
    }
    for (i, v) in st.imu.gyro.iter().enumerate() {
        put(&mut r, 40 + i * 2, *v);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn report_layout() {
        let mut st = State::default().with(Button::A).with(Button::Steam);
        st.axes = [100, -200, 255, 0, 0, 0];
        st.hat = [1, 0];
        st.left_pad.x = 7;
        st.left_pad.y = -8;
        st.imu.accel = [9, 0, 0];
        let r = state_report(7, 0, &st);
        assert_eq!(r[0], 0x42);
        assert_eq!(r[2] & 1, 1); // A
        assert_eq!(r[3] & 0x08, 0x08); // dpad right
        assert_eq!(r[4] & 1, 1); // Steam
        assert_eq!(i16::from_le_bytes([r[6], r[7]]), 32767); // left trigger
        assert_eq!(r[5] & 0x08, 0x08); // left trigger full pull
        assert_eq!(i16::from_le_bytes([r[12], r[13]]), 200);
        assert_eq!(i16::from_le_bytes([r[18], r[19]]), 7); // left pad X
        assert_eq!(i16::from_le_bytes([r[34], r[35]]), 9); // accel
    }

    /// Idle `0x42` report captured from a real controller: the fields we fill in
    /// must land at the same offsets with the same encoding.
    #[test]
    fn matches_captured_real_report() {
        const REAL: &str = "42d1000000300000000088fa54010cfe80fe000000000000000000000000c032210075037917cf3bb5ffefff0b007178390dbcfbf9d6";
        let real: Vec<u8> = (0..REAL.len() / 2)
            .map(|i| u8::from_str_radix(&REAL[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        // Sticks as sent over the wire are down-positive; the device is up-positive.
        let mut st = State::default();
        st.axes = [-1400, -340, 0, -500, 384, 0];
        st.imu = Imu {
            accel: [885, 6009, 15311],
            gyro: [-75, -17, 11],
        };
        let ours = state_report(0xd1, 2175680, &st);
        assert_eq!(real.len(), ours.len());
        assert_eq!(
            &ours[..30],
            &real[..30],
            "header, buttons, triggers, sticks, pads"
        );
        assert_eq!(&ours[30..46], &real[30..46], "timestamp, accel and gyro");
    }

    #[test]
    fn imu_data_is_zeroed_while_the_host_has_it_off() {
        let mut st = State::default();
        st.imu = Imu {
            accel: [1, 2, 3],
            gyro: [4, 5, 6],
        };
        assert_eq!(gate_imu(&st, true), st);
        let off = gate_imu(&st, false);
        assert_eq!(off.imu, Imu::default());
    }

    #[test]
    fn the_streamer_numbers_reports_and_timestamps_them_from_the_clock_passed_in() {
        let t0 = Instant::now();
        let mut streamer = Streamer::new(t0);
        let st = State::default();
        let first = streamer.tick(t0 + Duration::from_millis(4), &st, true);
        let second = streamer.tick(t0 + Duration::from_millis(8), &st, true);
        assert_eq!((first[1], second[1]), (1, 2), "sequence number counts up");
        let ts = |r: [u8; REPORT_LEN]| u32::from_le_bytes([r[30], r[31], r[32], r[33]]);
        assert_eq!(
            (ts(first), ts(second)),
            (4_000, 8_000),
            "microseconds since start"
        );
    }

    #[test]
    fn the_streamer_zeroes_motion_data_while_the_host_has_it_off() {
        let t0 = Instant::now();
        let mut streamer = Streamer::new(t0);
        let mut st = State::default();
        st.imu.accel = [300, -400, 16000];
        st.imu.gyro = [5, 6, 7];
        let on = streamer.tick(t0, &st, true);
        let off = streamer.tick(t0, &st, false);
        assert_eq!(i16::from_le_bytes([on[34], on[35]]), 300);
        assert_eq!(&off[34..46], &[0u8; 12], "accel and gyro bytes are zero");
    }

    #[test]
    fn the_report_sequence_byte_wraps_like_the_real_device() {
        let t0 = Instant::now();
        let mut streamer = Streamer::new(t0);
        let st = State::default();
        let mut last = [0u8; REPORT_LEN];
        for _ in 0..256 {
            last = streamer.tick(t0, &st, true);
        }
        assert_eq!(last[1], 0, "the 256th report wraps the one-byte counter");
    }
}
