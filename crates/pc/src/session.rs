//! Who the server is listening to: decides which datagrams count, and when the
//! controller must be released. No sockets and no clock, so it is tested by
//! feeding it packets.

use dvc_common::controller::State;
use dvc_common::proto::{self, DecodeError, Packet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// What to do with a datagram.
#[derive(Debug, PartialEq)]
pub enum Accept {
    /// Apply this state. `new_client` is true for the first packet from a sender.
    State {
        state: State,
        new_client: bool,
        seq: u32,
    },
    /// A repeat of the packet last accepted (the Deck's heartbeat): nothing to
    /// apply, but the client is alive and worth acking.
    Heartbeat { seq: u32 },
    /// A late packet from the current client, older than one already accepted: ignore it.
    Stale,
    /// Not usable. `first` is true the first time this sender sends this kind of
    /// bad packet, so callers can log it once instead of per packet.
    Rejected { reason: DecodeError, first: bool },
}

pub struct Session {
    client: Option<SocketAddr>,
    last_seq: Option<u32>,
    last_rejection: Option<(SocketAddr, DecodeError)>,
    /// Ack the client at most this often (see [`Session::ack_due`]).
    ack_interval: Duration,
    last_ack: Option<Instant>,
    last_imu_wanted: bool,
}

impl Session {
    pub fn new(ack_interval: Duration) -> Self {
        Session {
            client: None,
            last_seq: None,
            last_rejection: None,
            ack_interval,
            last_ack: None,
            last_imu_wanted: true,
        }
    }

    /// Forget when the client was last acked, so its next packet is acked at once.
    fn reset_acks(&mut self) {
        self.last_ack = None;
        self.last_imu_wanted = true;
    }

    /// Whether to ack the current client now: once per interval, and straight
    /// away when the host's answer about motion data has changed since the last
    /// ack. A new client always gets its first ack immediately.
    pub fn ack_due(&mut self, now: Instant, imu_wanted: bool) -> bool {
        let due = imu_wanted != self.last_imu_wanted
            || self
                .last_ack
                .is_none_or(|t| now.duration_since(t) >= self.ack_interval);
        if due {
            self.last_ack = Some(now);
            self.last_imu_wanted = imu_wanted;
        }
        due
    }

    /// Handle one datagram. A packet from a different sender takes over the
    /// session, whatever its sequence number (the Deck restarted its client).
    pub fn accept(&mut self, from: SocketAddr, bytes: &[u8]) -> Accept {
        let packet = match Packet::try_decode(bytes) {
            Ok(p) => p,
            Err(reason) => {
                let first = self.last_rejection != Some((from, reason));
                self.last_rejection = Some((from, reason));
                return Accept::Rejected { reason, first };
            }
        };
        let new_client = self.client != Some(from);
        let fresh = self
            .last_seq
            .is_none_or(|last| proto::is_newer(packet.seq, last));
        if !new_client && !fresh {
            return if self.last_seq == Some(packet.seq) {
                Accept::Heartbeat { seq: packet.seq }
            } else {
                Accept::Stale
            };
        }
        if new_client {
            self.reset_acks();
        }
        self.client = Some(from);
        self.last_seq = Some(packet.seq);
        self.last_rejection = None;
        Accept::State {
            state: packet.state,
            new_client,
            seq: packet.seq,
        }
    }

    /// Nothing was heard for the timeout. Returns whether a client was connected
    /// (so it can be logged); the caller must release all inputs either way.
    pub fn on_silence(&mut self) -> bool {
        self.last_seq = None;
        self.reset_acks();
        self.client.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvc_common::controller::Button;

    /// A session that never acks on its own timer within a test.
    fn session() -> Session {
        Session::new(Duration::from_secs(60))
    }

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([192, 168, 1, 5], port))
    }

    fn packet(seq: u32, button: Option<Button>) -> Vec<u8> {
        let state = button.map_or(State::default(), |b| State::default().with(b));
        Packet { seq, state }.encode().to_vec()
    }

    fn state_of(a: Accept) -> (State, bool) {
        match a {
            Accept::State {
                state, new_client, ..
            } => (state, new_client),
            other => panic!("expected State, got {other:?}"),
        }
    }

    #[test]
    fn first_packet_connects_the_sender() {
        let mut s = session();
        let (state, new) = state_of(s.accept(addr(1), &packet(10, Some(Button::A))));
        assert!(new && state.pressed(Button::A));
    }

    #[test]
    fn newer_packets_apply_and_are_not_new_clients() {
        let mut s = session();
        s.accept(addr(1), &packet(10, None));
        let (state, new) = state_of(s.accept(addr(1), &packet(11, Some(Button::B))));
        assert!(!new && state.pressed(Button::B));
    }

    #[test]
    fn late_packets_are_stale_and_repeats_are_heartbeats() {
        let mut s = session();
        s.accept(addr(1), &packet(10, None));
        assert_eq!(
            s.accept(addr(1), &packet(10, None)),
            Accept::Heartbeat { seq: 10 }
        );
        assert_eq!(s.accept(addr(1), &packet(9, None)), Accept::Stale);
    }

    #[test]
    fn sequence_numbers_wrap() {
        let mut s = session();
        s.accept(addr(1), &packet(u32::MAX, None));
        assert!(matches!(
            s.accept(addr(1), &packet(0, None)),
            Accept::State { .. }
        ));
    }

    #[test]
    fn a_new_sender_takes_over_even_with_a_lower_sequence() {
        let mut s = session();
        s.accept(addr(1), &packet(5000, None));
        let (_, new) = state_of(s.accept(addr(2), &packet(1, None)));
        assert!(new);
        // The old sender's packets are now from a "new" sender again, not stale.
        let (_, new) = state_of(s.accept(addr(1), &packet(5001, None)));
        assert!(new);
    }

    #[test]
    fn silence_resets_so_a_restarted_client_is_accepted() {
        let mut s = session();
        s.accept(addr(1), &packet(5000, None));
        assert!(s.on_silence());
        assert!(!s.on_silence(), "second silence has no client to report");
        let (_, new) = state_of(s.accept(addr(1), &packet(1, None)));
        assert!(new);
    }

    #[test]
    fn bad_packets_are_rejected_with_a_reason_and_reported_once() {
        let mut s = session();
        let mut old = vec![0u8; 24];
        old[..4].copy_from_slice(b"DVC1");
        assert_eq!(
            s.accept(addr(1), &old),
            Accept::Rejected {
                reason: DecodeError::Version(b'1'),
                first: true
            }
        );
        assert_eq!(
            s.accept(addr(1), &old),
            Accept::Rejected {
                reason: DecodeError::Version(b'1'),
                first: false
            }
        );
        // A different reason, or a different sender, is reported again.
        assert_eq!(
            s.accept(addr(1), b"junk"),
            Accept::Rejected {
                reason: DecodeError::NotOurs,
                first: true
            }
        );
        assert_eq!(
            s.accept(addr(2), b"junk"),
            Accept::Rejected {
                reason: DecodeError::NotOurs,
                first: true
            }
        );
    }

    #[test]
    fn rejected_packets_do_not_disturb_the_session() {
        let mut s = session();
        s.accept(addr(1), &packet(10, None));
        s.accept(addr(1), b"junk");
        assert_eq!(
            s.accept(addr(1), &packet(10, None)),
            Accept::Heartbeat { seq: 10 }
        );
    }

    #[test]
    fn acks_are_due_once_per_interval() {
        let mut s = Session::new(Duration::from_secs(1));
        let t0 = Instant::now();
        assert!(s.ack_due(t0, true), "the first ack is immediate");
        assert!(!s.ack_due(t0 + Duration::from_millis(999), true));
        assert!(s.ack_due(t0 + Duration::from_secs(1), true));
    }

    #[test]
    fn a_change_in_the_hosts_imu_wish_is_acked_at_once() {
        let mut s = session();
        let t0 = Instant::now();
        assert!(s.ack_due(t0, true));
        assert!(
            !s.ack_due(t0 + Duration::from_millis(10), true),
            "nothing changed"
        );
        assert!(
            s.ack_due(t0 + Duration::from_millis(20), false),
            "the host turned the gyro off"
        );
        assert!(!s.ack_due(t0 + Duration::from_millis(30), false));
        assert!(
            s.ack_due(t0 + Duration::from_millis(40), true),
            "and back on"
        );
    }

    #[test]
    fn a_new_client_is_acked_at_once_even_inside_the_previous_clients_interval() {
        let mut s = session();
        let t0 = Instant::now();
        s.accept(addr(1), &packet(1, None));
        assert!(s.ack_due(t0, true));
        assert!(!s.ack_due(t0 + Duration::from_millis(5), true));

        s.accept(addr(2), &packet(1, None)); // another Deck takes over
        assert!(
            s.ack_due(t0 + Duration::from_millis(10), true),
            "the new client must not wait out the old one's interval"
        );
    }

    #[test]
    fn a_client_that_comes_back_after_silence_is_acked_at_once() {
        let mut s = session();
        let t0 = Instant::now();
        s.accept(addr(1), &packet(1, None));
        assert!(s.ack_due(t0, true));
        s.on_silence();
        s.accept(addr(1), &packet(1, None));
        assert!(s.ack_due(t0 + Duration::from_millis(5), true));
    }

    #[test]
    fn repeat_packets_from_the_same_client_do_not_reset_the_ack_timer() {
        let mut s = session();
        let t0 = Instant::now();
        s.accept(addr(1), &packet(1, None));
        assert!(s.ack_due(t0, true));
        s.accept(addr(1), &packet(1, None)); // a heartbeat
        s.accept(addr(1), &packet(2, None));
        assert!(!s.ack_due(t0 + Duration::from_millis(5), true));
    }
}
