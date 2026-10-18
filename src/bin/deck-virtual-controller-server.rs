//! Runs on the PC: receives controller state over UDP and presents it as an
//! emulated Steam Controller (via /dev/uhid) or an Xbox pad (via /dev/uinput).

use serde::Deserialize;
use std::net::UdpSocket;
use std::time::Duration;
use steamdeck_as_pc_controller::config::Validate;
use steamdeck_as_pc_controller::proto;
use steamdeck_as_pc_controller::serve::{Acker, Event, serve_once};
use steamdeck_as_pc_controller::session::Session;
use steamdeck_as_pc_controller::sink::Sink;
use steamdeck_as_pc_controller::startup::{EXIT_ENV, die, load_config};
use steamdeck_as_pc_controller::steam_controller::{DeviceEvent, SteamSink};
use steamdeck_as_pc_controller::xbox_sink::XboxSink;

const APP: &str = "deck-virtual-controller-server";

#[derive(Deserialize)]
#[serde(default)]
struct Config {
    bind_addr: String,
    /// Release all inputs if nothing is heard from the client for this long.
    timeout_ms: u64,
    /// "xbox" (Xbox 360 pad) or "steam" (Valve Steam Controller).
    controller_type: ControllerType,
    /// Name of the virtual gamepad.
    name: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind_addr: format!("0.0.0.0:{}", proto::DEFAULT_PORT),
            timeout_ms: 1000,
            controller_type: ControllerType::Steam,
            name: "Steam Deck Virtual Controller".into(),
        }
    }
}

impl Validate for Config {}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ControllerType {
    /// uinput Xbox 360 gamepad.
    Xbox,
    /// Emulated 2026 Steam Controller (28de:1302) over /dev/uhid: buttons, sticks, trackpads, gyro.
    Steam,
}

fn open_sink(cfg: &Config) -> Box<dyn Sink> {
    match cfg.controller_type {
        ControllerType::Xbox => Box::new(XboxSink::create(&cfg.name).unwrap_or_else(|e| {
            die(
                EXIT_ENV,
                &format!("cannot create uinput device: {e} (need write access to /dev/uinput)"),
            )
        })),
        ControllerType::Steam => {
            let (tx, rx) = std::sync::mpsc::channel();
            let sink = SteamSink::create_with_events(&cfg.name, tx).unwrap_or_else(|e| {
                die(
                    EXIT_ENV,
                    &format!("cannot create uhid device: {e} (need write access to /dev/uhid)"),
                )
            });
            // Rumble and trackpad pulses are not forwarded to the Deck yet.
            std::thread::spawn(move || {
                for event in rx {
                    if let DeviceEvent::ImuEnabled(on) = event {
                        eprintln!("host turned the gyro {}", if on { "on" } else { "off" });
                    }
                }
            });
            Box::new(sink)
        }
    }
}

fn main() {
    let cfg: Config = load_config(APP);

    let mut sink = open_sink(&cfg);
    let sock = UdpSocket::bind(&cfg.bind_addr)
        .unwrap_or_else(|e| die(EXIT_ENV, &format!("cannot bind {}: {e}", cfg.bind_addr)));
    sock.set_read_timeout(Some(Duration::from_millis(cfg.timeout_ms.max(50))))
        .unwrap_or_else(|e| die(EXIT_ENV, &format!("cannot set socket timeout: {e}")));
    eprintln!(
        "listening on {}, virtual device '{}'",
        cfg.bind_addr, cfg.name
    );

    let mut session = Session::new();
    let mut acker = Acker::new(Duration::from_millis(250));
    loop {
        match serve_once(&sock, &mut session, sink.as_mut(), &mut acker) {
            Event::Connected(from) => eprintln!("client connected: {from}"),
            Event::Rejected { from, reason } => {
                eprintln!("ignoring packets from {from}: {reason}")
            }
            Event::TimedOut {
                was_connected: true,
            } => {
                eprintln!("client timed out, releasing inputs")
            }
            Event::RecvError(e) => eprintln!("recv error: {e}"),
            Event::Applied | Event::Stale | Event::RejectedAgain | Event::TimedOut { .. } => {}
        }
    }
}
