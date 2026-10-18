//! The human-readable log: one line per event, on stderr (which is what the
//! systemd journal and Steam's game log collect). Libraries call [`info!`],
//! [`warn!`] and [`error!`]; they never decide where lines go or how they look,
//! and tests can [`capture`] what was logged.
//!
//! This is separate from the machine-readable status lines the client prints on
//! stdout for the Deck app (see `dvc_deck::status`).

use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Normal progress: connected, started, switched.
    Info,
    /// Something is off but the program carries on.
    Warn,
    /// Something failed.
    Error,
}

thread_local! {
    static CAPTURED: RefCell<Option<Vec<(Level, String)>>> = const { RefCell::new(None) };
}

/// The line as written to stderr: info is the bare message; warnings and errors
/// say what they are.
pub fn format_line(level: Level, message: &str) -> String {
    match level {
        Level::Info => message.to_string(),
        Level::Warn => format!("warning: {message}"),
        Level::Error => format!("error: {message}"),
    }
}

/// Write one log line (or record it, inside [`capture`]).
pub fn emit(level: Level, message: String) {
    let captured = CAPTURED.with(|c| match c.borrow_mut().as_mut() {
        Some(lines) => {
            lines.push((level, message.clone()));
            true
        }
        None => false,
    });
    if !captured {
        eprintln!("{}", format_line(level, &message));
    }
}

/// Run `f` and return what the current thread logged meanwhile (instead of
/// printing it). Lines logged on other threads are not captured.
pub fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<(Level, String)>) {
    CAPTURED.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let result = f();
    let lines = CAPTURED.with(|c| c.borrow_mut().take().unwrap_or_default());
    (result, lines)
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::log::emit($crate::log::Level::Info, format!($($arg)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::log::emit($crate::log::Level::Warn, format!($($arg)*)) };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::log::emit($crate::log::Level::Error, format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_is_bare_and_warnings_and_errors_say_what_they_are() {
        assert_eq!(format_line(Level::Info, "listening"), "listening");
        assert_eq!(format_line(Level::Warn, "no data"), "warning: no data");
        assert_eq!(
            format_line(Level::Error, "emit failed"),
            "error: emit failed"
        );
    }

    #[test]
    fn capture_returns_what_was_logged_in_order_with_levels() {
        let (value, lines) = capture(|| {
            crate::info!("started {}", 1);
            crate::warn!("careful");
            crate::error!("broke: {}", "badly");
            42
        });
        assert_eq!(value, 42);
        assert_eq!(
            lines,
            [
                (Level::Info, "started 1".to_string()),
                (Level::Warn, "careful".to_string()),
                (Level::Error, "broke: badly".to_string()),
            ]
        );
    }

    #[test]
    fn nothing_is_captured_outside_capture() {
        let (_, inside) = capture(|| crate::info!("one"));
        assert_eq!(inside.len(), 1);
        // A later capture starts empty: the earlier lines did not leak into it.
        let (_, again) = capture(|| ());
        assert!(again.is_empty());
    }

    #[test]
    fn capture_only_sees_its_own_thread() {
        let (_, lines) = capture(|| {
            std::thread::spawn(|| crate::info!("from another thread"))
                .join()
                .unwrap();
            crate::info!("from this thread");
        });
        assert_eq!(lines, [(Level::Info, "from this thread".to_string())]);
    }
}
