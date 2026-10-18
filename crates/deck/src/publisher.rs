//! The client's publishing policy: which controller states become packets, how
//! they are numbered, when they are repeated, and when the user asked to stop.
//! Sources (the Deck hardware, an evdev pad) feed it states; it knows nothing
//! about where they come from or how packets travel.

use dvc_common::controller::{Imu, State, axis};
use dvc_common::proto::Packet;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Where packets go.
pub trait Transport {
    fn send(&mut self, packet: &Packet);
}

/// A connected UDP socket. Send errors are ignored: the next packet supersedes.
impl Transport for UdpSocket {
    fn send(&mut self, packet: &Packet) {
        let _ = UdpSocket::send(self, &packet.encode());
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A packet went out.
    Sent,
    /// Nothing worth sending changed.
    Quiet,
    /// View + Menu were pressed together. The controller has been released.
    Stop,
}

/// How stick values are cleaned up before they are compared and sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tuning {
    /// Raw stick units: smaller stick values read as zero.
    pub deadzone: i32,
    /// Raw stick units: a stick that moved less than this since the last value
    /// sent counts as not having moved. A resting stick jitters by a unit or two
    /// on most reports, which would otherwise be sent every time.
    pub stick_noise: i32,
}

pub struct Publisher<T: Transport> {
    transport: T,
    tuning: Tuning,
    current: State,
    seq: u32,
    /// Whether the server can use motion sensor data. Starts true; the server's
    /// acks switch it. While false the IMU is zeroed, so sensor noise no longer
    /// makes every report look like a change.
    imu_wanted: Arc<AtomicBool>,
}

impl<T: Transport> Publisher<T> {
    pub fn new(transport: T, tuning: Tuning) -> Self {
        Publisher {
            transport,
            tuning,
            current: State::default(),
            seq: 0,
            imu_wanted: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The flag the server's acks control; share it with whatever reads them.
    pub fn imu_wanted_flag(&self) -> Arc<AtomicBool> {
        self.imu_wanted.clone()
    }

    /// Take the latest state from a source. Sends a packet if it differs from
    /// the previous one; on the stop chord, releases everything and says so.
    pub fn publish(&mut self, mut state: State) -> Outcome {
        for i in axis::STICKS {
            if (state.axes[i] as i32).abs() < self.tuning.deadzone {
                state.axes[i] = 0;
            }
            let last = self.current.axes[i];
            if (state.axes[i] as i32 - last as i32).abs() < self.tuning.stick_noise {
                state.axes[i] = last;
            }
        }
        if !self.imu_wanted.load(Ordering::Relaxed) {
            state.imu = Imu::default();
        }
        if state.is_stop_chord() {
            self.current = State::default();
            self.seq = self.seq.wrapping_add(1);
            self.send_current();
            return Outcome::Stop;
        }
        if state == self.current {
            return Outcome::Quiet;
        }
        self.current = state;
        self.seq = self.seq.wrapping_add(1);
        self.send_current();
        Outcome::Sent
    }

    /// Repeat the current state (same sequence number) so the server knows the
    /// Deck is still there.
    pub fn heartbeat(&mut self) {
        self.send_current();
    }

    fn send_current(&mut self) {
        self.transport.send(&Packet {
            seq: self.seq,
            state: self.current,
        });
    }
}

/// Why [`run`] returned.
#[derive(Debug, PartialEq, Eq)]
pub enum RunEnd {
    /// The user asked to stop; the release was sent several times.
    Stopped,
    /// Every source went away.
    SourcesGone,
}

/// Publish every state arriving on `states`, repeating the current one every
/// `heartbeat` while the sources are quiet.
pub fn run<T: Transport>(
    states: &Receiver<State>,
    publisher: &mut Publisher<T>,
    heartbeat: Duration,
) -> RunEnd {
    // A heartbeat is due `heartbeat` after the last packet of any kind, whether or
    // not reports keep arriving: the Deck delivers one every few milliseconds even
    // when nothing changes, and the server releases the controller after a second
    // of silence.
    let mut last_sent = Instant::now();
    loop {
        match states.recv_timeout(heartbeat.saturating_sub(last_sent.elapsed())) {
            Ok(state) => match publisher.publish(state) {
                Outcome::Stop => {
                    // UDP can drop packets; make sure the release gets through.
                    for _ in 0..2 {
                        std::thread::sleep(Duration::from_millis(20));
                        publisher.heartbeat();
                    }
                    return RunEnd::Stopped;
                }
                Outcome::Sent => last_sent = Instant::now(),
                Outcome::Quiet => {}
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return RunEnd::SourcesGone,
        }
        if last_sent.elapsed() >= heartbeat {
            publisher.heartbeat();
            last_sent = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvc_common::controller::Button;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Collects the packets it is asked to send.
    #[derive(Clone, Default)]
    struct Recorder(Rc<RefCell<Vec<Packet>>>);

    impl Transport for Recorder {
        fn send(&mut self, packet: &Packet) {
            self.0.borrow_mut().push(*packet);
        }
    }

    fn publisher(deadzone: i32) -> (Publisher<Recorder>, Recorder) {
        publisher_tuned(Tuning {
            deadzone,
            stick_noise: 0,
        })
    }

    fn publisher_tuned(tuning: Tuning) -> (Publisher<Recorder>, Recorder) {
        let rec = Recorder::default();
        (Publisher::new(rec.clone(), tuning), rec)
    }

    fn pressed(b: Button) -> State {
        State::default().with(b)
    }

    #[test]
    fn a_changed_state_is_sent_with_the_next_sequence_number() {
        let (mut p, rec) = publisher(0);
        assert_eq!(p.publish(pressed(Button::A)), Outcome::Sent);
        p.publish(pressed(Button::B));
        let sent = rec.0.borrow();
        assert_eq!(sent.len(), 2);
        assert_eq!((sent[0].seq, sent[1].seq), (1, 2));
        assert!(sent[0].state.pressed(Button::A) && sent[1].state.pressed(Button::B));
    }

    #[test]
    fn an_unchanged_state_sends_nothing() {
        let (mut p, rec) = publisher(0);
        assert_eq!(p.publish(pressed(Button::A)), Outcome::Sent);
        assert_eq!(p.publish(pressed(Button::A)), Outcome::Quiet);
        assert_eq!(rec.0.borrow().len(), 1);
    }

    #[test]
    fn a_heartbeat_repeats_the_current_state_and_sequence_number() {
        let (mut p, rec) = publisher(0);
        p.publish(pressed(Button::A));
        p.heartbeat();
        let sent = rec.0.borrow();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], sent[1]);
    }

    #[test]
    fn a_heartbeat_before_any_input_sends_the_neutral_state() {
        let (mut p, rec) = publisher(0);
        p.heartbeat();
        assert_eq!(
            rec.0.borrow()[0],
            Packet {
                seq: 0,
                state: State::default()
            }
        );
    }

    #[test]
    fn small_stick_values_are_zeroed_but_triggers_are_not() {
        let (mut p, rec) = publisher(100);
        let mut s = State::default();
        s.axes[axis::LEFT_X] = 99;
        s.axes[axis::RIGHT_Y] = -100;
        s.axes[axis::LEFT_TRIGGER] = 5;
        p.publish(s);
        let sent = rec.0.borrow();
        assert_eq!(sent[0].state.axes[axis::LEFT_X], 0);
        assert_eq!(
            sent[0].state.axes[axis::RIGHT_Y],
            -100,
            "at the deadzone edge is outside it"
        );
        assert_eq!(sent[0].state.axes[axis::LEFT_TRIGGER], 5);
    }

    #[test]
    fn jitter_inside_the_deadzone_does_not_send() {
        let (mut p, rec) = publisher(50);
        let mut s = State::default();
        s.axes[axis::LEFT_X] = 10;
        p.publish(s);
        s.axes[axis::LEFT_X] = -20;
        p.publish(s);
        assert!(rec.0.borrow().is_empty(), "both read as the neutral state");
    }

    #[test]
    fn the_stop_chord_releases_everything_and_stops() {
        let (mut p, rec) = publisher(0);
        p.publish(pressed(Button::A));
        let chord = pressed(Button::A).with(Button::View).with(Button::Menu);
        assert_eq!(p.publish(chord), Outcome::Stop);
        let sent = rec.0.borrow();
        assert_eq!(sent.last().unwrap().state, State::default());
        assert!(
            sent.last().unwrap().seq > sent[0].seq,
            "the release is a new packet"
        );
    }

    #[test]
    fn run_sends_heartbeats_while_quiet_and_repeats_the_release_on_stop() {
        let (mut p, rec) = publisher(0);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(pressed(Button::A)).unwrap();
        tx.send(pressed(Button::View).with(Button::Menu)).unwrap();
        assert_eq!(run(&rx, &mut p, Duration::from_millis(10)), RunEnd::Stopped);
        let sent = rec.0.borrow();
        let neutral = sent
            .iter()
            .filter(|pk| pk.state == State::default())
            .count();
        assert_eq!(neutral, 3, "the release is sent three times");
    }

    #[test]
    fn run_ends_when_every_source_is_gone() {
        let (mut p, rec) = publisher(0);
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        assert_eq!(
            run(&rx, &mut p, Duration::from_millis(10)),
            RunEnd::SourcesGone
        );
        assert!(rec.0.borrow().is_empty());
    }

    #[test]
    fn run_heartbeats_when_nothing_arrives() {
        let (mut p, rec) = publisher(0);
        let (tx, rx) = std::sync::mpsc::channel::<State>();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            drop(tx);
        });
        run(&rx, &mut p, Duration::from_millis(10));
        handle.join().unwrap();
        assert!(rec.0.borrow().len() >= 2, "several heartbeats in 60 ms");
    }

    fn with_imu(accel_x: i16) -> State {
        let mut s = State::default();
        s.imu.accel[0] = accel_x;
        s
    }

    #[test]
    fn imu_noise_is_sent_while_the_server_wants_it() {
        let (mut p, rec) = publisher(0);
        p.publish(with_imu(1));
        p.publish(with_imu(2));
        assert_eq!(rec.0.borrow().len(), 2);
        assert_eq!(rec.0.borrow()[1].state.imu.accel[0], 2);
    }

    #[test]
    fn imu_noise_stops_being_traffic_once_the_server_does_not_want_it() {
        let (mut p, rec) = publisher(0);
        p.imu_wanted_flag().store(false, Ordering::Relaxed);
        for noise in 1..100 {
            p.publish(with_imu(noise));
        }
        assert!(
            rec.0.borrow().is_empty(),
            "an idle Deck with the IMU off is silent"
        );
        p.publish(State::default().with(dvc_common::controller::Button::A));
        let sent = rec.0.borrow();
        assert_eq!(sent.len(), 1, "real input still goes out");
        assert_eq!(sent[0].state.imu, Imu::default());
    }

    #[test]
    fn switching_the_imu_back_on_resumes_sending_it() {
        let (mut p, rec) = publisher(0);
        let flag = p.imu_wanted_flag();
        flag.store(false, Ordering::Relaxed);
        p.publish(with_imu(5));
        flag.store(true, Ordering::Relaxed);
        p.publish(with_imu(6));
        assert_eq!(rec.0.borrow().last().unwrap().state.imu.accel[0], 6);
    }

    fn stick(x: i16) -> State {
        let mut s = State::default();
        s.axes[axis::LEFT_X] = x;
        s
    }

    fn noisy(stick_noise: i32) -> (Publisher<Recorder>, Recorder) {
        publisher_tuned(Tuning {
            deadzone: 0,
            stick_noise,
        })
    }

    #[test]
    fn a_resting_sticks_jitter_is_not_sent() {
        let (mut p, rec) = noisy(128);
        p.publish(stick(-280)); // its resting offset: a real change from 0
        for x in [-240, -320, -280, -300, -250, -330] {
            p.publish(stick(x)); // an 88-unit-wide jitter band, as on a real Deck
        }
        assert_eq!(rec.0.borrow().len(), 1, "only the first value went out");
    }

    #[test]
    fn real_movement_is_sent_and_the_resting_offset_is_not_flattened() {
        let (mut p, rec) = noisy(128);
        p.publish(stick(-280));
        p.publish(stick(-1000));
        p.publish(stick(-1010)); // jitter around the new position
        let sent = rec.0.borrow();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].state.axes[axis::LEFT_X], -280);
        assert_eq!(sent[1].state.axes[axis::LEFT_X], -1000);
    }

    #[test]
    fn noise_is_measured_from_the_last_value_sent_so_slow_drift_still_gets_through() {
        let (mut p, rec) = noisy(128);
        for x in 0..=384 {
            p.publish(stick(x)); // creeps by one unit per report
        }
        let sent: Vec<i16> = rec
            .0
            .borrow()
            .iter()
            .map(|pk| pk.state.axes[axis::LEFT_X])
            .collect();
        assert_eq!(
            sent,
            [128, 256, 384],
            "each step of at least 128 gets through"
        );
    }

    #[test]
    fn zero_noise_threshold_sends_every_change() {
        let (mut p, rec) = noisy(0);
        p.publish(stick(1));
        p.publish(stick(2));
        assert_eq!(rec.0.borrow().len(), 2);
    }

    #[test]
    fn triggers_are_not_noise_gated() {
        let (mut p, rec) = noisy(100);
        let mut s = State::default();
        s.axes[axis::RIGHT_TRIGGER] = 3;
        p.publish(s);
        assert_eq!(rec.0.borrow()[0].state.axes[axis::RIGHT_TRIGGER], 3);
    }

    #[test]
    fn an_idle_deck_with_the_gyro_off_is_silent() {
        // Both noise sources a real idle Deck has: stick jitter and motion sensors.
        let (mut p, rec) = noisy(128);
        p.imu_wanted_flag().store(false, Ordering::Relaxed);
        for i in 0..500i16 {
            let mut s = stick(-280 + (i % 89) - 44); // about the jitter of a real Deck
            s.axes[axis::RIGHT_Y] = 1900 + (i % 91) - 45;
            s.imu.accel = [i, -i, i];
            s.imu.gyro = [i % 7, i % 11, i % 13];
            p.publish(s);
        }
        assert!(
            rec.0.borrow().len() <= 2,
            "sent {} packets",
            rec.0.borrow().len()
        );
    }

    #[test]
    fn heartbeats_continue_while_unchanged_reports_keep_arriving() {
        // The Deck delivers a report every ~4 ms even when nothing changes. The
        // server must still hear a heartbeat (it releases the controller after a
        // second of silence), so heartbeats cannot depend on the reports stopping.
        let (mut p, rec) = publisher(0);
        let (tx, rx) = std::sync::mpsc::channel();
        let feeder = std::thread::spawn(move || {
            for _ in 0..60 {
                if tx.send(State::default()).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        run(&rx, &mut p, Duration::from_millis(20));
        feeder.join().unwrap();
        let heartbeats = rec.0.borrow().len();
        assert!(
            heartbeats >= 4,
            "only {heartbeats} heartbeats in ~120 ms of steady reports"
        );
    }
}
