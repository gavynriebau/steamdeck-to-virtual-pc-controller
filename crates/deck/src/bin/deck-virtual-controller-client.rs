//! Runs on the Steam Deck: reads the Deck's controller and streams its state to
//! the PC over UDP.

use dvc_common::config::Validate;
use dvc_common::proto;
use dvc_common::startup::{EXIT_CONFIG, EXIT_ENV, die, load_config};
use dvc_deck::publisher::{Publisher, RunEnd, run};
use dvc_deck::status::{EXIT_STOPPED_BY_USER, Status, print_to_stdout};
use dvc_deck::{deck_hid, evdev_source, link};
use serde::Deserialize;
use std::net::UdpSocket;
use std::time::Duration;

const APP: &str = "deck-virtual-controller-client";

/// Where Deck input is read from.
#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
enum Source {
    /// The Deck hardware directly (works in Game Mode with any Steam layout).
    Hidraw,
    /// A Linux gamepad device.
    Evdev,
}

#[derive(Deserialize)]
#[serde(default)]
struct Config {
    source: Source,
    /// Address of the PC running deck-virtual-controller-server.
    server_addr: String,
    /// (evdev source) evdev path (e.g. /dev/input/event10) or device name; auto-detected if empty.
    device: String,
    /// Resend interval so the server knows we are alive.
    heartbeat_ms: u64,
    /// Stick deadzone, in raw units (axes are -32768..32767).
    deadzone: i32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            source: Source::Hidraw,
            server_addr: format!("127.0.0.1:{}", proto::DEFAULT_PORT),
            device: String::new(),
            heartbeat_ms: 100,
            deadzone: 0,
        }
    }
}

impl Validate for Config {
    fn validate(&self) -> Result<(), String> {
        if self.server_addr.contains("PC_ADDRESS") {
            return Err(format!(
                "set server_addr in your config.toml (it still holds the placeholder '{}')",
                self.server_addr
            ));
        }
        Ok(())
    }
}

fn main() {
    let cfg: Config = load_config(APP);

    let sock = UdpSocket::bind("0.0.0.0:0")
        .unwrap_or_else(|e| die(EXIT_ENV, &format!("cannot create UDP socket: {e}")));
    sock.connect(&cfg.server_addr).unwrap_or_else(|e| {
        die(
            EXIT_CONFIG,
            &format!("bad server_addr '{}': {e}", cfg.server_addr),
        )
    });
    eprintln!("sending to {}", cfg.server_addr);
    let report = print_to_stdout();
    report(Status::Target(cfg.server_addr.clone()));

    match sock.try_clone() {
        Ok(listener) => {
            let report = report.clone();
            link::spawn_monitor(listener, Duration::from_secs(3), move |reachable| {
                if reachable {
                    eprintln!("server reachable");
                } else {
                    eprintln!("no reply from the server (check server_addr and the PC's firewall)");
                }
                report(Status::Server(reachable));
            })
        }
        Err(e) => eprintln!("cannot watch for server replies: {e}"),
    }

    let (tx, rx) = std::sync::mpsc::channel();
    match cfg.source {
        Source::Hidraw => deck_hid::spawn_reader(tx, report),
        Source::Evdev => evdev_source::spawn(cfg.device.clone(), tx, report),
    }

    let mut publisher = Publisher::new(sock, cfg.deadzone);
    let heartbeat = Duration::from_millis(cfg.heartbeat_ms.max(10));
    match run(&rx, &mut publisher, heartbeat) {
        RunEnd::Stopped => {
            // This exit status tells the Deck app to close too.
            eprintln!("stop requested (View + Menu)");
            std::process::exit(EXIT_STOPPED_BY_USER);
        }
        RunEnd::SourcesGone => die(EXIT_ENV, "the controller readers stopped unexpectedly"),
    }
}
