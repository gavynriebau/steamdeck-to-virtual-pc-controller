//! Runs the compiled server binary and checks its command line.

use std::process::{Command, Output};

const SERVER: &str = env!("CARGO_BIN_EXE_deck-virtual-controller-server");

fn run(args: &[&str]) -> Output {
    Command::new(SERVER)
        .args(args)
        .output()
        .expect("run the server")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn version_prints_the_name_version_and_protocol_and_exits_0() {
    let out = run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout(&out).trim(),
        format!(
            "deck-virtual-controller-server {} (protocol {})",
            env!("CARGO_PKG_VERSION"),
            char::from(dvc_common::proto::VERSION)
        )
    );
}

#[test]
fn help_describes_the_options_and_exits_0() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(
        text.contains("usage: deck-virtual-controller-server [--config <path>]"),
        "{text}"
    );
    assert!(
        text.contains("~/.config/deck-virtual-controller-server/config.toml"),
        "{text}"
    );
}

#[test]
fn an_unknown_flag_is_exit_2_and_points_at_help() {
    let out = run(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--bogus") && stderr.contains("--help"),
        "{stderr}"
    );
}
