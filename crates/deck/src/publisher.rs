//! The client's publishing policy: which controller states become packets, how
//! they are numbered, when they are repeated, and when the user asked to stop.
//! Sources (the Deck hardware, an evdev pad) feed it states; it knows nothing
//! about where they come from or how packets travel.

use dvc_common::controller::{State, axis};
use dvc_common::proto::Packet;
use std::net::UdpSocket;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

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
    Continue,
    /// View + Menu were pressed together. The controller has been released.
    Stop,
}

pub struct Publisher<T: Transport> {
    transport: T,
    deadzone: i32,
    current: State,
    seq: u32,
}

impl<T: Transport> Publisher<T> {
    /// `deadzone` is in raw stick units: smaller stick values read as zero.
    pub fn new(transport: T, deadzone: i32) -> Self {
        Publisher {
            transport,
            deadzone,
            current: State::default(),
            seq: 0,
        }
    }

    /// Take the latest state from a source. Sends a packet if it differs from
    /// the previous one; on the stop chord, releases everything and says so.
    pub fn publish(&mut self, mut state: State) -> Outcome {
        for i in axis::STICKS {
            if (state.axes[i] as i32).abs() < self.deadzone {
                state.axes[i] = 0;
            }
        }
        if state.is_stop_chord() {
            self.current = State::default();
            self.seq = self.seq.wrapping_add(1);
            self.send_current();
            return Outcome::Stop;
        }
        if state != self.current {
            self.current = state;
            self.seq = self.seq.wrapping_add(1);
            self.send_current();
        }
        Outcome::Continue
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
    loop {
        match states.recv_timeout(heartbeat) {
            Ok(state) => {
                if publisher.publish(state) == Outcome::Stop {
                    // UDP can drop packets; make sure the release gets through.
                    for _ in 0..2 {
                        std::thread::sleep(Duration::from_millis(20));
                        publisher.heartbeat();
                    }
                    return RunEnd::Stopped;
                }
            }
            Err(RecvTimeoutError::Timeout) => publisher.heartbeat(),
            Err(RecvTimeoutError::Disconnected) => return RunEnd::SourcesGone,
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
        let rec = Recorder::default();
        (Publisher::new(rec.clone(), deadzone), rec)
    }

    fn pressed(b: Button) -> State {
        State::default().with(b)
    }

    #[test]
    fn a_changed_state_is_sent_with_the_next_sequence_number() {
        let (mut p, rec) = publisher(0);
        assert_eq!(p.publish(pressed(Button::A)), Outcome::Continue);
        p.publish(pressed(Button::B));
        let sent = rec.0.borrow();
        assert_eq!(sent.len(), 2);
        assert_eq!((sent[0].seq, sent[1].seq), (1, 2));
        assert!(sent[0].state.pressed(Button::A) && sent[1].state.pressed(Button::B));
    }

    #[test]
    fn an_unchanged_state_sends_nothing() {
        let (mut p, rec) = publisher(0);
        p.publish(pressed(Button::A));
        p.publish(pressed(Button::A));
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
}
