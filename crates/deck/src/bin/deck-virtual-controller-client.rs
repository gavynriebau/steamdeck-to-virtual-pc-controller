//! Runs on the Steam Deck: reads the Deck's controller and streams its state to
//! the PC over UDP. The wiring lives in `dvc_deck::run`; this reads the config,
//! opens the socket, picks the input source and turns the outcome into an exit
//! status.

use dvc_common::info;
use dvc_common::startup::{EXIT_CONFIG, EXIT_ENV, die, load_config};
use dvc_deck::config::{Config, Source};
use dvc_deck::deck_hid::HidrawDiscovery;
use dvc_deck::evdev_source::EvdevDiscovery;
use dvc_deck::publisher::RunEnd;
use dvc_deck::run::{Settings, run_client};
use dvc_deck::source::Discovery;
use dvc_deck::status::{EXIT_STOPPED_BY_USER, print_to_stdout};
use std::net::UdpSocket;

const APP: &str = "deck-virtual-controller-client";

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

    let discovery: Box<dyn Discovery> = match cfg.source {
        Source::Hidraw => Box::new(HidrawDiscovery),
        Source::Evdev => Box::new(EvdevDiscovery {
            device: cfg.device.clone(),
        }),
    };

    match run_client(
        sock,
        discovery,
        print_to_stdout(),
        &Settings::from_config(&cfg),
    ) {
        Ok(RunEnd::Stopped) => {
            // This exit status tells the Deck app to close too.
            info!("stop requested (View + Menu)");
            std::process::exit(EXIT_STOPPED_BY_USER);
        }
        Ok(RunEnd::SourcesGone) => die(EXIT_ENV, "the controller readers stopped unexpectedly"),
        Err(e) => die(EXIT_ENV, &format!("cannot start the client: {e}")),
    }
}
