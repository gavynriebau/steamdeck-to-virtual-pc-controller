//! Runs on the PC: receives controller state over UDP and presents it as an
//! emulated Steam Controller (via /dev/uhid) or an Xbox pad (via /dev/uinput).
//! The receive loop lives in `dvc_pc::run`; this reads the config, creates the
//! real virtual device and binds the socket.

use dvc_common::info;
use dvc_common::startup::{AppInfo, EXIT_ENV, die, load_config};
use dvc_common::timing;
use dvc_pc::config::{Config, ControllerType};
use dvc_pc::run::{Timing, run_server};
use dvc_pc::sink::Sink;
use dvc_pc::steam_controller::SteamSink;
use dvc_pc::xbox_sink::XboxSink;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;

const INFO: AppInfo = AppInfo {
    name: "deck-virtual-controller-server",
    version: env!("CARGO_PKG_VERSION"),
    about: "Presents a Steam Deck's controls on this PC as a Steam Controller or an Xbox pad.",
};

fn open_sink(cfg: &Config) -> Box<dyn Sink> {
    match cfg.controller_type {
        ControllerType::Xbox => Box::new(XboxSink::create(&cfg.name).unwrap_or_else(|e| {
            die(
                EXIT_ENV,
                &format!("cannot create uinput device: {e} (need write access to /dev/uinput)"),
            )
        })),
        ControllerType::Steam => Box::new(SteamSink::create(&cfg.name).unwrap_or_else(|e| {
            die(
                EXIT_ENV,
                &format!("cannot create uhid device: {e} (need write access to /dev/uhid)"),
            )
        })),
    }
}

fn main() {
    let cfg: Config = load_config(&INFO);

    let mut sink = open_sink(&cfg);
    let sock = UdpSocket::bind(&cfg.bind_addr)
        .unwrap_or_else(|e| die(EXIT_ENV, &format!("cannot bind {}: {e}", cfg.bind_addr)));
    info!(
        "listening on {}, virtual device '{}'",
        cfg.bind_addr, cfg.name
    );

    let timing = Timing {
        silence_timeout: cfg.silence_timeout(),
        ack_interval: timing::ACK_INTERVAL,
    };
    // Nothing ever sets this: the server runs until it is killed.
    let never = AtomicBool::new(false);
    if let Err(e) = run_server(&sock, sink.as_mut(), &timing, &never) {
        die(EXIT_ENV, &format!("the server stopped: {e}"));
    }
}
