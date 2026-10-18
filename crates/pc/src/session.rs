//! Who the server is listening to: decides which datagrams count, and when the
//! controller must be released. No sockets and no clock, so it is tested by
//! feeding it packets.

use dvc_common::controller::State;
use dvc_common::proto::{self, DecodeError, Packet};
use std::net::SocketAddr;

/// What to do with a datagram.
#[derive(Debug, PartialEq)]
pub enum Accept {
    /// Apply this state. `new_client` is true for the first packet from a sender.
    State {
        state: State,
        new_client: bool,
        seq: u32,
    },
    /// A late or duplicate packet from the current client: ignore it.
    Stale,
    /// Not usable. `first` is true the first time this sender sends this kind of
    /// bad packet, so callers can log it once instead of per packet.
    Rejected { reason: DecodeError, first: bool },
}

#[derive(Default)]
pub struct Session {
    client: Option<SocketAddr>,
    last_seq: Option<u32>,
    last_rejection: Option<(SocketAddr, DecodeError)>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
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
            return Accept::Stale;
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
        self.client.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvc_common::controller::Button;

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
        let mut s = Session::new();
        let (state, new) = state_of(s.accept(addr(1), &packet(10, Some(Button::A))));
        assert!(new && state.pressed(Button::A));
    }

    #[test]
    fn newer_packets_apply_and_are_not_new_clients() {
        let mut s = Session::new();
        s.accept(addr(1), &packet(10, None));
        let (state, new) = state_of(s.accept(addr(1), &packet(11, Some(Button::B))));
        assert!(!new && state.pressed(Button::B));
    }

    #[test]
    fn late_and_duplicate_packets_are_stale() {
        let mut s = Session::new();
        s.accept(addr(1), &packet(10, None));
        assert_eq!(s.accept(addr(1), &packet(10, None)), Accept::Stale);
        assert_eq!(s.accept(addr(1), &packet(9, None)), Accept::Stale);
    }

    #[test]
    fn sequence_numbers_wrap() {
        let mut s = Session::new();
        s.accept(addr(1), &packet(u32::MAX, None));
        assert!(matches!(
            s.accept(addr(1), &packet(0, None)),
            Accept::State { .. }
        ));
    }

    #[test]
    fn a_new_sender_takes_over_even_with_a_lower_sequence() {
        let mut s = Session::new();
        s.accept(addr(1), &packet(5000, None));
        let (_, new) = state_of(s.accept(addr(2), &packet(1, None)));
        assert!(new);
        // The old sender's packets are now from a "new" sender again, not stale.
        let (_, new) = state_of(s.accept(addr(1), &packet(5001, None)));
        assert!(new);
    }

    #[test]
    fn silence_resets_so_a_restarted_client_is_accepted() {
        let mut s = Session::new();
        s.accept(addr(1), &packet(5000, None));
        assert!(s.on_silence());
        assert!(!s.on_silence(), "second silence has no client to report");
        let (_, new) = state_of(s.accept(addr(1), &packet(1, None)));
        assert!(new);
    }

    #[test]
    fn bad_packets_are_rejected_with_a_reason_and_reported_once() {
        let mut s = Session::new();
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
        let mut s = Session::new();
        s.accept(addr(1), &packet(10, None));
        s.accept(addr(1), b"junk");
        assert_eq!(s.accept(addr(1), &packet(10, None)), Accept::Stale);
    }
}
