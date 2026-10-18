//! Runs on the Steam Deck: reads the Deck's controller and streams its state to
//! the PC over UDP. The wiring lives in `dvc_deck::run`; this reads the config,
//! opens the socket, picks the input source and turns the outcome into an exit
//! status. Whenever it has to exit with an error it first tells the Deck app why
//! (a `status fatal=` line), because the Deck screen cannot see the log.

use dvc_common::info;
use dvc_common::startup::{AppInfo, EXIT_CONFIG, EXIT_ENV, die, try_load_config};
use dvc_deck::config::{Config, Source};
use dvc_deck::deck_hid::HidrawDiscovery;
use dvc_deck::evdev_source::EvdevDiscovery;
use dvc_deck::publisher::RunEnd;
use dvc_deck::run::{Settings, run_client};
use dvc_deck::source::Discovery;
use dvc_deck::status::{EXIT_STOPPED_BY_USER, Status, print_to_stdout};
use std::net::UdpSocket;

const INFO: AppInfo = AppInfo {
    name: "deck-virtual-controller-client",
    version: env!("CARGO_PKG_VERSION"),
    about: "Streams this Steam Deck's controls to a PC running deck-virtual-controller-server.",
};

/// Report `why` to the Deck app, then log it and exit with `code`.
fn fatal(code: i32, why: &str) -> ! {
    print_to_stdout()(Status::fatal(why));
    die(code, why)
}

fn main() {
    let cfg: Config = try_load_config(&INFO).unwrap_or_else(|e| fatal(EXIT_CONFIG, &e));

    let sock = UdpSocket::bind("0.0.0.0:0")
        .unwrap_or_else(|e| fatal(EXIT_ENV, &format!("cannot create UDP socket: {e}")));
    sock.connect(&cfg.server_addr).unwrap_or_else(|e| {
        fatal(
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
        Ok(RunEnd::SourcesGone) => fatal(EXIT_ENV, "the controller readers stopped unexpectedly"),
        Err(e) => fatal(EXIT_ENV, &format!("cannot start the client: {e}")),
    }
}
