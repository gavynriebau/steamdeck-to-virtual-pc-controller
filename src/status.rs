//! What the client tells the Deck app about itself.
//!
//! This is the whole interface between the two processes:
//!
//! * **stdout** carries one line per status change, `status <key>=<value>`:
//!   `target=<host:port>`, `controller=ok|missing`, `server=reachable|unreachable`.
//!   Anything else on stdout is ignored. Human-readable logs go to stderr.
//! * **Exit status** [`EXIT_STOPPED_BY_USER`] (0) means the user pressed
//!   View + Menu and the app should close too. Any other status is a failure and
//!   the app restarts the client.
//!
//! The Deck app (`deck/deck_supervisor.py`) is the other side of this contract.

use std::fmt;
use std::sync::Arc;

/// Exit status of a client the user asked to stop.
pub const EXIT_STOPPED_BY_USER: i32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Where the client is sending to.
    Target(String),
    /// Whether Deck controller data is arriving.
    Controller(bool),
    /// Whether the server's acks are arriving.
    Server(bool),
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Target(t) => write!(f, "status target={t}"),
            Status::Controller(true) => write!(f, "status controller=ok"),
            Status::Controller(false) => write!(f, "status controller=missing"),
            Status::Server(true) => write!(f, "status server=reachable"),
            Status::Server(false) => write!(f, "status server=unreachable"),
        }
    }
}

impl Status {
    /// Parse a line the client printed; `None` for anything that isn't a status.
    pub fn parse(line: &str) -> Option<Status> {
        let (key, value) = line.trim().strip_prefix("status ")?.split_once('=')?;
        match (key, value) {
            ("target", t) if !t.is_empty() => Some(Status::Target(t.to_string())),
            ("controller", "ok") => Some(Status::Controller(true)),
            ("controller", "missing") => Some(Status::Controller(false)),
            ("server", "reachable") => Some(Status::Server(true)),
            ("server", "unreachable") => Some(Status::Server(false)),
            _ => None,
        }
    }
}

/// Where status changes are reported. The client prints them; tests collect them.
pub type Report = Arc<dyn Fn(Status) + Send + Sync>;

/// A [`Report`] that prints each status on stdout and flushes, so the Deck app
/// sees it immediately.
pub fn print_to_stdout() -> Report {
    Arc::new(|status| {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{status}");
        let _ = out.flush();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_round_trips_through_its_line() {
        for s in [
            Status::Target("192.168.1.20:24800".into()),
            Status::Controller(true),
            Status::Controller(false),
            Status::Server(true),
            Status::Server(false),
        ] {
            assert_eq!(Status::parse(&s.to_string()), Some(s));
        }
    }

    #[test]
    fn lines_that_are_not_statuses_are_ignored() {
        for line in [
            "",
            "sending to 1.2.3.4:5",
            "status",
            "status server",
            "status server=maybe",
            "status colour=blue",
            "status target=",
            "statusserver=reachable",
        ] {
            assert_eq!(Status::parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        assert_eq!(
            Status::parse("  status server=reachable \n"),
            Some(Status::Server(true))
        );
    }

    #[test]
    fn the_lines_are_the_documented_wire_text() {
        // The Python supervisor matches these exact strings; change both together.
        assert_eq!(
            Status::Controller(false).to_string(),
            "status controller=missing"
        );
        assert_eq!(
            Status::Server(false).to_string(),
            "status server=unreachable"
        );
        assert_eq!(EXIT_STOPPED_BY_USER, 0);
    }
}
