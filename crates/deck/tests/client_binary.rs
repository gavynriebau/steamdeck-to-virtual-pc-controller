//! Runs the compiled client binary, the way the Deck app does, and checks what
//! it tells the app when it cannot start.

use std::io::Write;
use std::process::{Command, Output};

const CLIENT: &str = env!("CARGO_BIN_EXE_deck-virtual-controller-client");

fn run_with_config(toml: &str) -> Output {
    let dir = std::env::temp_dir().join(format!(
        "dvc-client-bin-{}-{}",
        std::process::id(),
        toml.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(toml.as_bytes())
        .unwrap();
    Command::new(CLIENT)
        .arg("--config")
        .arg(&path)
        .output()
        .expect("run the client")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_placeholder_address_exits_2_and_tells_the_app_what_to_fix() {
    let out = run_with_config("server_addr = \"PC_ADDRESS:24800\"\n");
    assert_eq!(
        out.status.code(),
        Some(2),
        "a config problem is exit 2: restarting cannot help"
    );
    let lines: Vec<String> = stdout(&out).lines().map(String::from).collect();
    let last = lines.last().expect("the client printed nothing on stdout");
    assert!(
        last.starts_with("status fatal=set server_addr in your config.toml"),
        "last stdout line was {last:?}"
    );
}

#[test]
fn an_unreadable_value_exits_2_with_the_parse_error_on_one_line() {
    let out = run_with_config("heartbeat_ms = \"fast\"\nserver_addr = \"127.0.0.1:1\"\n");
    assert_eq!(out.status.code(), Some(2));
    let last = stdout(&out).lines().last().unwrap().to_string();
    assert!(last.starts_with("status fatal="), "{last:?}");
    assert!(
        last.contains("heartbeat_ms"),
        "the reason names the setting: {last:?}"
    );
}

#[test]
fn a_heartbeat_the_server_would_not_tolerate_is_a_fatal_reason_too() {
    let out = run_with_config("server_addr = \"127.0.0.1:1\"\nheartbeat_ms = 2000\n");
    assert_eq!(out.status.code(), Some(2));
    assert!(stdout(&out).contains("status fatal=heartbeat_ms is 2000"));
}

#[test]
fn a_fatal_line_is_the_only_status_a_config_failure_prints() {
    let out = run_with_config("server_addr = \"PC_ADDRESS:1\"\n");
    let statuses: Vec<String> = stdout(&out)
        .lines()
        .filter(|l| l.starts_with("status "))
        .map(String::from)
        .collect();
    assert_eq!(statuses.len(), 1, "{statuses:?}");
}

fn run_with_args(args: &[&str]) -> Output {
    Command::new(CLIENT)
        .args(args)
        .output()
        .expect("run the client")
}

#[test]
fn version_prints_the_name_version_and_protocol_and_exits_0() {
    for flag in ["--version", "-V"] {
        let out = run_with_args(&[flag]);
        assert_eq!(out.status.code(), Some(0), "{flag}");
        assert_eq!(
            stdout(&out).trim(),
            format!(
                "deck-virtual-controller-client {} (protocol {})",
                env!("CARGO_PKG_VERSION"),
                char::from(dvc_common::proto::VERSION)
            )
        );
    }
}

#[test]
fn help_describes_the_options_and_exits_0() {
    for flag in ["--help", "-h"] {
        let out = run_with_args(&[flag]);
        assert_eq!(out.status.code(), Some(0), "{flag}");
        let text = stdout(&out);
        assert!(
            text.contains("usage: deck-virtual-controller-client [--config <path>]"),
            "{text}"
        );
        assert!(text.contains("--version"), "{text}");
    }
}

#[test]
fn an_unknown_flag_is_exit_2_and_points_at_help() {
    let out = run_with_args(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--bogus") && stderr.contains("--help"),
        "{stderr}"
    );
}

#[test]
fn the_first_log_line_names_the_version_so_a_bug_report_can_quote_it() {
    let out = run_with_config("server_addr = \"PC_ADDRESS:24800\"\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first = stderr.lines().next().unwrap_or("");
    assert_eq!(
        first,
        format!(
            "deck-virtual-controller-client {} (protocol {})",
            env!("CARGO_PKG_VERSION"),
            char::from(dvc_common::proto::VERSION)
        )
    );
}
