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
