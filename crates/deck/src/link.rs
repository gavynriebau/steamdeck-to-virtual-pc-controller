//! Does the server hear us? UDP never says, so the server acks now and then and
//! this tracks whether acks keep arriving.

use crate::status::ServerLink;
use dvc_common::proto::{Ack, DecodeError};
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

/// What one step of the monitor saw.
#[derive(Debug, PartialEq, Eq)]
pub struct Observation {
    /// The ack carried by the datagram, if it was one.
    pub ack: Option<Ack>,
    /// Set when what we know about the server changed.
    pub change: Option<ServerLink>,
}

/// One step of the monitor's loop: a datagram that arrived (or none within the
/// poll interval) and the time. The thread in [`spawn_monitor`] only supplies those.
pub struct Monitor {
    link: Link,
    /// The protocol version byte of the last ack that was refused for its version.
    incompatible_with: Option<u8>,
}

impl Monitor {
    pub fn new(now: Instant, timeout: Duration) -> Self {
        Monitor {
            link: Link::new(now, timeout),
            incompatible_with: None,
        }
    }

    pub fn observe(&mut self, datagram: Option<&[u8]>, now: Instant) -> Observation {
        let mut ack = None;
        let mut change = None;
        match datagram.map(Ack::try_decode) {
            Some(Ok(a)) => {
                self.incompatible_with = None;
                self.link.ack_received(now);
                ack = Some(a);
            }
            // A server answering in another protocol version is there, but not
            // usable: say so once instead of letting it look unreachable.
            Some(Err(DecodeError::Version(v))) => {
                if self.incompatible_with != Some(v) {
                    self.incompatible_with = Some(v);
                    change = Some(ServerLink::Incompatible);
                }
            }
            Some(Err(_)) | None => {}
        }
        if let Some(reachable) = self.link.check(now)
            && self.incompatible_with.is_none()
        {
            change = Some(if reachable {
                ServerLink::Reachable
            } else {
                ServerLink::Unreachable
            });
        }
        Observation { ack, change }
    }
}

/// Watch the (connected) UDP socket for acks in the background and call
/// `on_change` when the server becomes reachable, unreachable or incompatible,
/// and `on_ack` for every ack.
pub fn spawn_monitor(
    sock: UdpSocket,
    timeout: Duration,
    mut on_change: impl FnMut(ServerLink) + Send + 'static,
    mut on_ack: impl FnMut(Ack) + Send + 'static,
) {
    let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
    std::thread::spawn(move || {
        let mut monitor = Monitor::new(Instant::now(), timeout);
        let mut buf = [0u8; 16];
        loop {
            // Errors (timeouts, ICMP "port unreachable") just mean no ack.
            let datagram = sock.recv(&mut buf).ok().map(|n| &buf[..n]);
            let seen = monitor.observe(datagram, Instant::now());
            if let Some(ack) = seen.ack {
                on_ack(ack);
            }
            if let Some(link) = seen.change {
                on_change(link);
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

    fn ack_bytes(seq: u32, imu_wanted: bool) -> [u8; Ack::LEN] {
        Ack { seq, imu_wanted }.encode()
    }

    #[test]
    fn an_ack_is_handed_on_and_makes_the_server_reachable() {
        let t0 = Instant::now();
        let mut m = Monitor::new(t0, T);
        let seen = m.observe(Some(&ack_bytes(7, false)), at(t0, 0.1));
        assert_eq!(
            seen,
            Observation {
                ack: Some(Ack {
                    seq: 7,
                    imu_wanted: false
                }),
                change: Some(ServerLink::Reachable)
            }
        );
    }

    #[test]
    fn other_datagrams_are_not_acks_and_do_not_count() {
        let t0 = Instant::now();
        let mut m = Monitor::new(t0, T);
        let seen = m.observe(Some(b"hello"), at(t0, 0.1));
        assert_eq!(
            seen,
            Observation {
                ack: None,
                change: None
            }
        );
        let seen = m.observe(None, at(t0, 3.5));
        assert_eq!(
            seen.change,
            Some(ServerLink::Unreachable),
            "junk never counted as a sign of life"
        );
    }

    #[test]
    fn silence_turns_reachable_into_unreachable_and_an_ack_turns_it_back() {
        let t0 = Instant::now();
        let mut m = Monitor::new(t0, T);
        m.observe(Some(&ack_bytes(1, true)), at(t0, 0.1));
        assert_eq!(
            m.observe(None, at(t0, 3.3)).change,
            Some(ServerLink::Unreachable)
        );
        assert_eq!(m.observe(None, at(t0, 4.0)).change, None, "reported once");
        assert_eq!(
            m.observe(Some(&ack_bytes(2, true)), at(t0, 4.5)).change,
            Some(ServerLink::Reachable)
        );
    }

    fn ack_bytes_with_version(version: u8) -> [u8; Ack::LEN] {
        let mut b = ack_bytes(1, true);
        b[3] = version;
        b
    }

    #[test]
    fn a_server_with_another_protocol_version_is_incompatible_not_unreachable() {
        let t0 = Instant::now();
        let mut m = Monitor::new(t0, T);
        let old = ack_bytes_with_version(b'2');
        let seen = m.observe(Some(&old), at(t0, 0.1));
        assert_eq!(
            seen,
            Observation {
                ack: None,
                change: Some(ServerLink::Incompatible)
            }
        );
        assert_eq!(
            m.observe(Some(&old), at(t0, 0.4)).change,
            None,
            "reported once"
        );
        // However long it goes on, it never turns into 'unreachable'.
        assert_eq!(m.observe(None, at(t0, 10.0)).change, None);
    }

    #[test]
    fn a_matching_ack_after_a_mismatch_makes_the_server_reachable() {
        let t0 = Instant::now();
        let mut m = Monitor::new(t0, T);
        m.observe(Some(&ack_bytes_with_version(b'2')), at(t0, 0.1));
        let seen = m.observe(Some(&ack_bytes(5, true)), at(t0, 0.5));
        assert_eq!(seen.change, Some(ServerLink::Reachable));
        assert_eq!(seen.ack.map(|a| a.seq), Some(5));
    }

    #[test]
    fn a_new_mismatch_after_recovery_is_reported_again() {
        let t0 = Instant::now();
        let mut m = Monitor::new(t0, T);
        m.observe(Some(&ack_bytes_with_version(b'2')), at(t0, 0.1));
        m.observe(Some(&ack_bytes(1, true)), at(t0, 0.2));
        let seen = m.observe(Some(&ack_bytes_with_version(b'9')), at(t0, 0.3));
        assert_eq!(seen.change, Some(ServerLink::Incompatible));
    }
}
