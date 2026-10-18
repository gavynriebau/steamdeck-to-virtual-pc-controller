//! The server's wiring: the receive loop that joins the socket, the session,
//! the sink and the acks, and what it logs. The binary only reads config and
//! creates the real virtual device; this runs the same loop against any sink,
//! so it can be driven by a real client in tests.

use crate::serve::{Acker, Event, serve_once};
use crate::session::Session;
use crate::sink::Sink;
use dvc_common::{info, warn};
use std::io;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct Timing {
    /// Release the controller after this long without hearing the Deck.
    pub silence_timeout: Duration,
    /// Ack the Deck at most this often.
    pub ack_interval: Duration,
}

/// Serve the Deck on `sock` until `stop` is set. The stop flag is checked at
/// least once per `silence_timeout`.
pub fn run_server(
    sock: &UdpSocket,
    sink: &mut dyn Sink,
    timing: &Timing,
    stop: &AtomicBool,
) -> io::Result<()> {
    sock.set_read_timeout(Some(timing.silence_timeout))?;
    let mut session = Session::new();
    let mut acker = Acker::new(timing.ack_interval);
    while !stop.load(Ordering::Relaxed) {
        match serve_once(sock, &mut session, sink, &mut acker) {
            Event::Connected(from) => info!("client connected: {from}"),
            Event::Rejected { from, reason } => {
                warn!("ignoring packets from {from}: {reason}")
            }
            Event::TimedOut {
                was_connected: true,
            } => info!("client timed out, releasing inputs"),
            Event::RecvError(e) => warn!("recv error: {e}"),
            Event::Applied
            | Event::Heartbeat
            | Event::Stale
            | Event::RejectedAgain
            | Event::TimedOut { .. } => {}
        }
    }
    Ok(())
}
