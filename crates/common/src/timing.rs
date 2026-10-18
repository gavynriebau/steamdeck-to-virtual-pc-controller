//! The durations that the Deck and the PC must agree on, and the order they
//! have to stay in. The two sides run on different machines and read different
//! config files, so nothing else makes them consistent: this module says what
//! must be smaller than what, and the tests hold the defaults to it.
//!
//! ```text
//! heartbeat  x2 <=  server silence timeout     one lost packet must not drop the Deck
//! max(heartbeat, ack interval)  x2 <=  link timeout   a quiet link must not look dead
//! ```
//!
//! * The Deck repeats its state every **heartbeat**, so the server hears it even
//!   when nothing changes.
//! * The server releases the controller after the **silence timeout** with
//!   nothing heard.
//! * The server acks the Deck at most every **ack interval**, and only when it
//!   hears from it, so acks are no more frequent than heartbeats.
//! * The Deck calls the server unreachable after the **link timeout** without an ack.

use std::time::Duration;

/// How often the Deck repeats its state when nothing changes.
pub const HEARTBEAT: Duration = Duration::from_millis(100);
/// The smallest heartbeat worth configuring.
pub const MIN_HEARTBEAT: Duration = Duration::from_millis(10);
/// How long the server waits in silence before releasing the controller.
pub const SERVER_SILENCE_TIMEOUT: Duration = Duration::from_millis(1000);
/// The least the server may wait: below this even the default heartbeat can't keep up.
pub const MIN_SERVER_SILENCE_TIMEOUT: Duration = Duration::from_millis(200);
/// How often, at most, the server tells the Deck it is being heard.
pub const ACK_INTERVAL: Duration = Duration::from_millis(250);
/// How long without an ack before the Deck calls the server unreachable.
pub const LINK_TIMEOUT: Duration = Duration::from_secs(3);
/// How often the Deck looks for input devices that appeared or went away.
pub const SOURCE_POLL: Duration = Duration::from_secs(2);

/// The slowest heartbeat that survives a server using the default silence timeout.
pub fn max_heartbeat() -> Duration {
    SERVER_SILENCE_TIMEOUT / 2
}

/// Why a heartbeat is unusable, or `Ok` if it keeps the Deck connected to a
/// server using the default silence timeout.
pub fn check_heartbeat(heartbeat: Duration) -> Result<(), String> {
    if heartbeat < MIN_HEARTBEAT {
        return Err(format!(
            "heartbeat_ms is {} but must be at least {}: a faster heartbeat only floods the network",
            heartbeat.as_millis(),
            MIN_HEARTBEAT.as_millis()
        ));
    }
    if heartbeat > max_heartbeat() {
        return Err(format!(
            "heartbeat_ms is {} but must be at most {}: the server releases the controller after {} ms of silence, \
             so a slower heartbeat makes it connect and drop the Deck over and over",
            heartbeat.as_millis(),
            max_heartbeat().as_millis(),
            SERVER_SILENCE_TIMEOUT.as_millis()
        ));
    }
    Ok(())
}

/// Why a silence timeout is unusable, or `Ok` if the default heartbeat
/// (twice over) fits inside it.
pub fn check_silence_timeout(timeout: Duration) -> Result<(), String> {
    if timeout < MIN_SERVER_SILENCE_TIMEOUT {
        return Err(format!(
            "timeout_ms is {} but must be at least {}: the Deck's heartbeat is {} ms, \
             and one lost packet would already release the controller",
            timeout.as_millis(),
            MIN_SERVER_SILENCE_TIMEOUT.as_millis(),
            HEARTBEAT.as_millis()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_keep_the_documented_order() {
        assert!(
            HEARTBEAT * 2 <= SERVER_SILENCE_TIMEOUT,
            "heartbeat x2 <= silence timeout"
        );
        assert!(
            HEARTBEAT.max(ACK_INTERVAL) * 2 <= LINK_TIMEOUT,
            "max(heartbeat, ack interval) x2 <= link timeout"
        );
        assert!(MIN_HEARTBEAT <= HEARTBEAT);
        assert!(MIN_SERVER_SILENCE_TIMEOUT <= SERVER_SILENCE_TIMEOUT);
        assert!(
            HEARTBEAT * 2 <= MIN_SERVER_SILENCE_TIMEOUT,
            "the smallest allowed timeout fits two default heartbeats"
        );
    }

    #[test]
    fn the_default_heartbeat_and_timeout_pass_their_own_checks() {
        assert!(check_heartbeat(HEARTBEAT).is_ok());
        assert!(check_silence_timeout(SERVER_SILENCE_TIMEOUT).is_ok());
    }

    #[test]
    fn the_heartbeat_limits_are_inclusive() {
        assert!(check_heartbeat(MIN_HEARTBEAT).is_ok());
        assert!(check_heartbeat(max_heartbeat()).is_ok());
        assert!(check_heartbeat(MIN_HEARTBEAT - Duration::from_millis(1)).is_err());
        assert!(check_heartbeat(max_heartbeat() + Duration::from_millis(1)).is_err());
    }

    #[test]
    fn a_too_slow_heartbeat_explains_the_server_timeout() {
        let e = check_heartbeat(Duration::from_millis(2000)).unwrap_err();
        assert!(
            e.contains("2000") && e.contains("500") && e.contains("1000"),
            "{e}"
        );
    }

    #[test]
    fn a_too_small_silence_timeout_is_refused_with_the_heartbeat_in_the_message() {
        let e = check_silence_timeout(Duration::from_millis(50)).unwrap_err();
        assert!(
            e.contains("50") && e.contains("200") && e.contains("100"),
            "{e}"
        );
        assert!(check_silence_timeout(MIN_SERVER_SILENCE_TIMEOUT).is_ok());
    }
}
