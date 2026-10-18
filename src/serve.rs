//! The server's receive loop body: one datagram (or one silence) in, one
//! decision out. [`serve_once`] is what the server binary calls in a loop, and
//! what the loopback test drives.

use crate::controller::State;
use crate::proto::DecodeError;
use crate::session::{Accept, Session};
use crate::sink::Sink;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};

#[derive(Debug)]
pub enum Event {
    /// First packet from this sender; its state was applied.
    Connected(SocketAddr),
    /// A normal packet; its state was applied.
    Applied,
    /// A late or duplicate packet; nothing was applied.
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

/// Wait for one datagram (the socket's read timeout bounds the wait) and feed
/// it to the session and sink.
pub fn serve_once(sock: &UdpSocket, session: &mut Session, sink: &mut dyn Sink) -> Event {
    let mut buf = [0u8; 64];
    match sock.recv_from(&mut buf) {
        Ok((n, from)) => match session.accept(from, &buf[..n]) {
            Accept::State { state, new_client } => {
                sink.apply(&state);
                if new_client {
                    Event::Connected(from)
                } else {
                    Event::Applied
                }
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
    use crate::controller::Button;
    use crate::proto::Packet;
    use crate::sink::MemorySink;
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

    fn send(client: &UdpSocket, seq: u32, button: Option<Button>) {
        let state = button.map_or(State::default(), |b| State::default().with(b));
        client.send(&Packet { seq, state }.encode()).unwrap();
    }

    #[test]
    fn udp_to_session_to_sink() {
        let (server, client) = pair();
        let (mut session, mut sink) = (Session::new(), MemorySink::default());

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
        let (mut session, mut sink) = (Session::new(), MemorySink::default());
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
        let (mut session, mut sink) = (Session::new(), MemorySink::default());
        send(&client, 5, None);
        serve_once(&server, &mut session, &mut sink);
        send(&client, 5, Some(Button::A)); // duplicate sequence number
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
}
