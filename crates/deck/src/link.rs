//! Does the server hear us? UDP never says, so the server acks now and then and
//! this tracks whether acks keep arriving.

use dvc_common::proto::Ack;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

/// Reachability from the acks seen so far. Pure: the clock is passed in.
pub struct Link {
    timeout: Duration,
    started: Instant,
    last_ack: Option<Instant>,
    reported: Option<bool>,
}

impl Link {
    pub fn new(now: Instant, timeout: Duration) -> Self {
        Link {
            timeout,
            started: now,
            last_ack: None,
            reported: None,
        }
    }

    pub fn ack_received(&mut self, now: Instant) {
        self.last_ack = Some(now);
    }

    /// `Some(reachable)` when the answer changed since the last call. Before the
    /// first ack the answer is unknown until the timeout has passed.
    pub fn check(&mut self, now: Instant) -> Option<bool> {
        let reachable = match self.last_ack {
            Some(t) => Some(now.duration_since(t) <= self.timeout),
            None if now.duration_since(self.started) > self.timeout => Some(false),
            None => None,
        };
        if reachable.is_some() && reachable != self.reported {
            self.reported = reachable;
            return reachable;
        }
        None
    }
}

/// Watch the (connected) UDP socket for acks in the background and call
/// `on_change(true/false)` when the server becomes reachable or unreachable,
/// and `on_ack` for every ack.
pub fn spawn_monitor(
    sock: UdpSocket,
    timeout: Duration,
    mut on_change: impl FnMut(bool) + Send + 'static,
    mut on_ack: impl FnMut(Ack) + Send + 'static,
) {
    let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
    std::thread::spawn(move || {
        let mut link = Link::new(Instant::now(), timeout);
        let mut buf = [0u8; 16];
        loop {
            // Errors (timeouts, ICMP "port unreachable") just mean no ack.
            if let Ok(n) = sock.recv(&mut buf)
                && let Some(ack) = Ack::decode(&buf[..n])
            {
                link.ack_received(Instant::now());
                on_ack(ack);
            }
            if let Some(reachable) = link.check(Instant::now()) {
                on_change(reachable);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Duration = Duration::from_secs(3);

    fn at(t0: Instant, secs: f64) -> Instant {
        t0 + Duration::from_secs_f64(secs)
    }

    #[test]
    fn unknown_until_the_timeout_then_unreachable() {
        let t0 = Instant::now();
        let mut l = Link::new(t0, T);
        assert_eq!(l.check(at(t0, 1.0)), None);
        assert_eq!(l.check(at(t0, 3.5)), Some(false));
        assert_eq!(l.check(at(t0, 4.0)), None, "reported once");
    }

    #[test]
    fn the_first_ack_makes_it_reachable() {
        let t0 = Instant::now();
        let mut l = Link::new(t0, T);
        l.ack_received(at(t0, 0.5));
        assert_eq!(l.check(at(t0, 0.6)), Some(true));
        assert_eq!(l.check(at(t0, 1.0)), None);
    }

    #[test]
    fn acks_stopping_makes_it_unreachable_and_resuming_makes_it_reachable_again() {
        let t0 = Instant::now();
        let mut l = Link::new(t0, T);
        l.ack_received(at(t0, 0.1));
        assert_eq!(l.check(at(t0, 0.2)), Some(true));
        assert_eq!(l.check(at(t0, 3.2)), Some(false));
        l.ack_received(at(t0, 5.0));
        assert_eq!(l.check(at(t0, 5.1)), Some(true));
    }

    #[test]
    fn a_late_first_ack_beats_the_unreachable_report() {
        let t0 = Instant::now();
        let mut l = Link::new(t0, T);
        l.ack_received(at(t0, 2.9));
        assert_eq!(l.check(at(t0, 3.5)), Some(true));
    }
}
