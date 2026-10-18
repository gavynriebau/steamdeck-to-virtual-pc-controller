//! The server's receive loop body: one datagram (or one silence) in, one
//! decision out. [`serve_once`] is what the server binary calls in a loop, and
//! what the loopback test drives.

use crate::session::{Accept, Session};
use crate::sink::Sink;
use dvc_common::controller::State;
use dvc_common::proto::{self, Ack, DecodeError};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

#[derive(Debug)]
pub enum Event {
    /// First packet from this sender; its state was applied.
    Connected(SocketAddr),
    /// A normal packet; its state was applied.
    Applied,
    /// A repeat of the last packet (the Deck's heartbeat); nothing was applied.
    Heartbeat,
    /// A late packet; nothing was applied.
    Stale,
    /// A bad packet, reported once per sender and kind; nothing was applied.
    Rejected {
        from: SocketAddr,
        reason: DecodeError,
    },
    /// A bad packet already reported before.
    RejectedAgain,
    /// The Deck went quiet; the controller was released.
    TimedOut { was_connected: bool },
    /// The socket failed.
    RecvError(std::io::Error),
}

fn send_ack_if_due(
    sock: &UdpSocket,
    to: SocketAddr,
    seq: u32,
    sink: &dyn Sink,
    session: &mut Session,
) {
    let imu_wanted = sink.wants_imu();
    if session.ack_due(Instant::now(), imu_wanted) {
        let _ = sock.send_to(&Ack { seq, imu_wanted }.encode(), to);
    }
}

/// Wait for one datagram (the socket's read timeout bounds the wait) and feed
/// it to the session and sink.
pub fn serve_once(sock: &UdpSocket, session: &mut Session, sink: &mut dyn Sink) -> Event {
    let mut buf = [0u8; proto::MAX_DATAGRAM];
    match sock.recv_from(&mut buf) {
        Ok((n, from)) => match session.accept(from, &buf[..n]) {
            Accept::State {
                state,
                new_client,
                seq,
            } => {
                sink.apply(&state);
                send_ack_if_due(sock, from, seq, sink, session);
                if new_client {
                    Event::Connected(from)
                } else {
                    Event::Applied
                }
            }
            Accept::Heartbeat { seq } => {
                // A quiet Deck sends only heartbeats, and still needs its acks.
                send_ack_if_due(sock, from, seq, sink, session);
                Event::Heartbeat
            }
            Accept::Stale => Event::Stale,
            Accept::Rejected {
                reason,
                first: true,
            } => Event::Rejected { from, reason },
            Accept::Rejected { first: false, .. } => Event::RejectedAgain,
        },
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            let was_connected = session.on_silence();
            sink.apply(&State::default());
            Event::TimedOut { was_connected }
        }
        Err(e) => Event::RecvError(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::MemorySink;
    use dvc_common::controller::Button;
    use dvc_common::proto::Packet;
    use std::time::Duration;

    /// A server socket on loopback with a short silence timeout, and a client socket.
    fn pair() -> (UdpSocket, UdpSocket) {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client.connect(server.local_addr().unwrap()).unwrap();
        (server, client)
    }

    fn fixture() -> (Session, MemorySink) {
        (Session::new(Duration::from_secs(60)), MemorySink::default())
    }

    fn send(client: &UdpSocket, seq: u32, button: Option<Button>) {
        let state = button.map_or(State::default(), |b| State::default().with(b));
        client.send(&Packet { seq, state }.encode()).unwrap();
    }

    #[test]
    fn udp_to_session_to_sink() {
        let (server, client) = pair();
        let (mut session, mut sink) = fixture();

        send(&client, 1, Some(Button::A));
        assert!(matches!(
            serve_once(&server, &mut session, &mut sink),
            Event::Connected(_)
        ));
        send(&client, 2, Some(Button::B));
        assert!(matches!(
            serve_once(&server, &mut session, &mut sink),
            Event::Applied
        ));

        assert_eq!(sink.applied.len(), 2);
        assert!(sink.applied[0].pressed(Button::A));
        assert!(sink.applied[1].pressed(Button::B) && !sink.applied[1].pressed(Button::A));
    }

    #[test]
    fn silence_releases_the_controller() {
        let (server, client) = pair();
        let (mut session, mut sink) = fixture();
        send(&client, 1, Some(Button::A));
        serve_once(&server, &mut session, &mut sink);

        let event = serve_once(&server, &mut session, &mut sink);
        assert!(
            matches!(
                event,
                Event::TimedOut {
                    was_connected: true
                }
            ),
            "{event:?}"
        );
        assert_eq!(sink.applied.last(), Some(&State::default()));
    }

    #[test]
    fn stale_and_bad_packets_apply_nothing() {
        let (server, client) = pair();
        let (mut session, mut sink) = fixture();
        send(&client, 5, None);
        serve_once(&server, &mut session, &mut sink);
        send(&client, 3, Some(Button::A)); // older than one already accepted
        assert!(matches!(
            serve_once(&server, &mut session, &mut sink),
            Event::Stale
        ));
        client.send(b"DVC1 old client").unwrap();
        assert!(matches!(
            serve_once(&server, &mut session, &mut sink),
            Event::Rejected {
                reason: DecodeError::Version(b'1'),
                ..
            }
        ));
        assert_eq!(sink.applied.len(), 1);
    }

    #[test]
    fn the_sender_is_acked_once_per_interval_with_the_accepted_sequence() {
        let (server, client) = pair();
        client
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let (mut session, mut sink) = fixture();

        send(&client, 7, None);
        serve_once(&server, &mut session, &mut sink);
        let mut buf = [0u8; 16];
        let n = client.recv(&mut buf).unwrap();
        assert_eq!(
            Ack::decode(&buf[..n]),
            Some(Ack {
                seq: 7,
                imu_wanted: true
            })
        );

        // Within the interval: applied, but no second ack.
        send(&client, 8, None);
        serve_once(&server, &mut session, &mut sink);
        assert!(client.recv(&mut buf).is_err(), "no ack inside the interval");
        assert_eq!(sink.applied.len(), 2);
    }

    #[test]
    fn stale_and_bad_packets_are_not_acked() {
        let (server, client) = pair();
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let (mut session, mut sink) = fixture();
        client.send(b"junk").unwrap();
        serve_once(&server, &mut session, &mut sink);
        let mut buf = [0u8; 16];
        assert!(client.recv(&mut buf).is_err());
    }

    #[test]
    fn the_ack_tells_the_deck_what_the_sink_wants() {
        let (server, client) = pair();
        client
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let (mut session, mut sink) = fixture();
        sink.imu_wanted = false;
        send(&client, 1, None);
        serve_once(&server, &mut session, &mut sink);
        let mut buf = [0u8; 16];
        let n = client.recv(&mut buf).unwrap();
        assert!(!Ack::decode(&buf[..n]).unwrap().imu_wanted);
    }

    #[test]
    fn a_quiet_deck_sending_only_heartbeats_keeps_getting_acks() {
        let (server, client) = pair();
        client
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let (mut session, mut sink) = (
            Session::new(Duration::from_millis(30)),
            MemorySink::default(),
        );
        let mut buf = [0u8; 16];

        send(&client, 4, None);
        serve_once(&server, &mut session, &mut sink);
        client.recv(&mut buf).unwrap(); // the first ack

        // The same packet again and again, like the Deck's heartbeat at rest.
        let mut acks = 0;
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(35));
            send(&client, 4, None);
            assert!(matches!(
                serve_once(&server, &mut session, &mut sink),
                Event::Heartbeat
            ));
            if let Ok(n) = client.recv(&mut buf) {
                assert_eq!(Ack::decode(&buf[..n]).unwrap().seq, 4);
                acks += 1;
            }
        }
        assert!(acks >= 3, "only {acks} acks for 4 heartbeats");
        assert_eq!(sink.applied.len(), 1, "heartbeats apply nothing");
    }

    #[test]
    fn a_second_deck_connecting_inside_the_ack_interval_is_acked_straight_away() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let addr = server.local_addr().unwrap();
        let first = UdpSocket::bind("127.0.0.1:0").unwrap();
        let second = UdpSocket::bind("127.0.0.1:0").unwrap();
        for c in [&first, &second] {
            c.connect(addr).unwrap();
            c.set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
        }
        let (mut session, mut sink) = fixture(); // a 60 s ack interval

        send(&first, 1, None);
        serve_once(&server, &mut session, &mut sink);
        let mut buf = [0u8; 16];
        first.recv(&mut buf).expect("the first Deck is acked");

        send(&second, 1, None);
        serve_once(&server, &mut session, &mut sink);
        second
            .recv(&mut buf)
            .expect("the second Deck must not wait out the first one's interval");
    }
}
