//! Runs on the PC: receives controller state over UDP and presents it as an
//! emulated Steam Controller (via /dev/uhid) or an Xbox pad (via /dev/uinput).

use dvc_common::startup::{EXIT_ENV, die, load_config};
use dvc_pc::config::{Config, ControllerType};
use dvc_pc::serve::{Acker, Event, serve_once};
use dvc_pc::session::Session;
use dvc_pc::sink::Sink;
use dvc_pc::steam_controller::{DeviceEvent, SteamSink};
use dvc_pc::xbox_sink::XboxSink;
use std::net::UdpSocket;
use std::time::Duration;

const APP: &str = "deck-virtual-controller-server";

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
            Event::Applied
            | Event::Heartbeat
            | Event::Stale
            | Event::RejectedAgain
            | Event::TimedOut { .. } => {}
        }
    }
}
