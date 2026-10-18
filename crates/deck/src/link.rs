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

/// What one step of the monitor saw.
#[derive(Debug, PartialEq, Eq)]
pub struct Observation {
    /// The ack carried by the datagram, if it was one.
    pub ack: Option<Ack>,
    /// `Some(reachable)` when the server became reachable or unreachable.
    pub change: Option<bool>,
}

/// One step of the monitor's loop: a datagram that arrived (or none within the
/// poll interval) and the time. The thread in [`spawn_monitor`] only supplies those.
pub struct Monitor {
    link: Link,
}

impl Monitor {
    pub fn new(now: Instant, timeout: Duration) -> Self {
        Monitor {
            link: Link::new(now, timeout),
        }
    }

    pub fn observe(&mut self, datagram: Option<&[u8]>, now: Instant) -> Observation {
        let ack = datagram.and_then(Ack::decode);
        if ack.is_some() {
            self.link.ack_received(now);
        }
        Observation {
            ack,
            change: self.link.check(now),
        }
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
        let mut monitor = Monitor::new(Instant::now(), timeout);
        let mut buf = [0u8; 16];
        loop {
            // Errors (timeouts, ICMP "port unreachable") just mean no ack.
            let datagram = sock.recv(&mut buf).ok().map(|n| &buf[..n]);
            let seen = monitor.observe(datagram, Instant::now());
            if let Some(ack) = seen.ack {
                on_ack(ack);
            }
            if let Some(reachable) = seen.change {
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
                change: Some(true)
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
            Some(false),
            "junk never counted as a sign of life"
        );
    }

    #[test]
    fn silence_turns_reachable_into_unreachable_and_an_ack_turns_it_back() {
        let t0 = Instant::now();
        let mut m = Monitor::new(t0, T);
        m.observe(Some(&ack_bytes(1, true)), at(t0, 0.1));
        assert_eq!(m.observe(None, at(t0, 3.3)).change, Some(false));
        assert_eq!(m.observe(None, at(t0, 4.0)).change, None, "reported once");
        assert_eq!(
            m.observe(Some(&ack_bytes(2, true)), at(t0, 4.5)).change,
            Some(true)
        );
    }
}
