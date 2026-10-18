//! What the client tells the Deck app about itself.
//!
//! This is the whole interface between the two processes:
//!
//! * **stdout** carries one line per status change, `status <key>=<value>`:
//!   `target=<host:port>`, `controller=ok|missing`, `server=reachable|unreachable|incompatible`.
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

/// What the Deck knows about the server it sends to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLink {
    /// Acks arrive.
    Reachable,
    /// No acks for a while.
    Unreachable,
    /// The server answers, but with another protocol version.
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Where the client is sending to.
    Target(String),
    /// Whether Deck controller data is arriving.
    Controller(bool),
    /// Whether the server's acks are arriving, and speak our protocol version.
    Server(ServerLink),
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Target(t) => write!(f, "status target={t}"),
            Status::Controller(true) => write!(f, "status controller=ok"),
            Status::Controller(false) => write!(f, "status controller=missing"),
            Status::Server(ServerLink::Reachable) => write!(f, "status server=reachable"),
            Status::Server(ServerLink::Unreachable) => write!(f, "status server=unreachable"),
            Status::Server(ServerLink::Incompatible) => write!(f, "status server=incompatible"),
        }
    }
}

impl Status {
    /// The status as the `(key, value)` pair on its line.
    pub fn key_value(&self) -> (&'static str, String) {
        match self {
            Status::Target(t) => ("target", t.clone()),
            Status::Controller(true) => ("controller", "ok".into()),
            Status::Controller(false) => ("controller", "missing".into()),
            Status::Server(ServerLink::Reachable) => ("server", "reachable".into()),
            Status::Server(ServerLink::Unreachable) => ("server", "unreachable".into()),
            Status::Server(ServerLink::Incompatible) => ("server", "incompatible".into()),
        }
    }

    /// Parse a line the client printed; `None` for anything that isn't a status.
    pub fn parse(line: &str) -> Option<Status> {
        let (key, value) = line.trim().strip_prefix("status ")?.split_once('=')?;
        match (key, value) {
            ("target", t) if !t.is_empty() => Some(Status::Target(t.to_string())),
            ("controller", "ok") => Some(Status::Controller(true)),
            ("controller", "missing") => Some(Status::Controller(false)),
            ("server", "reachable") => Some(Status::Server(ServerLink::Reachable)),
            ("server", "unreachable") => Some(Status::Server(ServerLink::Unreachable)),
            ("server", "incompatible") => Some(Status::Server(ServerLink::Incompatible)),
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
            Status::Server(ServerLink::Reachable),
            Status::Server(ServerLink::Unreachable),
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
            Some(Status::Server(ServerLink::Reachable))
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
            Status::Server(ServerLink::Unreachable).to_string(),
            "status server=unreachable"
        );
        assert_eq!(EXIT_STOPPED_BY_USER, 0);
    }

    /// `contract/status_contract.txt` is shared with the Python Deck app's tests.
    const CONTRACT: &str = include_str!("../../../contract/status_contract.txt");

    fn rows(kind: &str) -> Vec<Vec<&'static str>> {
        CONTRACT
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.strip_prefix(kind)?.strip_prefix('|'))
            .map(|rest| rest.split('|').collect())
            .collect()
    }

    #[test]
    fn contract_valid_lines_parse_to_the_documented_key_and_value() {
        let valid = rows("valid");
        assert!(valid.len() >= 5, "the fixture lists every status");
        for row in valid {
            let status =
                Status::parse(row[0]).unwrap_or_else(|| panic!("{:?} should parse", row[0]));
            assert_eq!(
                status.key_value(),
                (row[1], row[2].to_string()),
                "{}",
                row[0]
            );
        }
    }

    #[test]
    fn contract_valid_lines_are_exactly_what_the_client_prints() {
        for row in rows("valid") {
            let status = Status::parse(row[0]).unwrap();
            assert_eq!(status.to_string(), row[0]);
        }
    }

    #[test]
    fn contract_ignored_lines_are_not_statuses() {
        let ignored = rows("ignored");
        assert!(!ignored.is_empty());
        for row in ignored {
            assert_eq!(Status::parse(row[0]), None, "{:?}", row[0]);
        }
    }

    #[test]
    fn every_status_the_client_can_print_is_in_the_contract() {
        let listed: Vec<&str> = rows("valid").iter().map(|r| r[0]).collect();
        for s in [
            Status::Controller(true),
            Status::Controller(false),
            Status::Server(ServerLink::Reachable),
            Status::Server(ServerLink::Unreachable),
            Status::Server(ServerLink::Incompatible),
        ] {
            assert!(
                listed.contains(&s.to_string().as_str()),
                "{s} missing from the contract"
            );
        }
        assert!(listed.iter().any(|l| l.starts_with("status target=")));
    }

    #[test]
    fn contract_exit_status_matches() {
        let exits = rows("exit");
        let stopped = exits.iter().find(|r| r[0] == "stopped-by-user").unwrap();
        assert_eq!(stopped[1].parse::<i32>().unwrap(), EXIT_STOPPED_BY_USER);
    }
}
