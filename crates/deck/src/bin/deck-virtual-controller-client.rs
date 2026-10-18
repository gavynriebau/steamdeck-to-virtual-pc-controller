//! Runs on the Steam Deck: reads the Deck's controller and streams its state to
//! the PC over UDP.

use dvc_common::startup::{EXIT_CONFIG, EXIT_ENV, die, load_config};
use dvc_common::timing;
use dvc_deck::config::{Config, Source};
use dvc_deck::deck_hid::HidrawDiscovery;
use dvc_deck::evdev_source::EvdevDiscovery;
use dvc_deck::link;
use dvc_deck::publisher::{Publisher, RunEnd, Tuning, run};
use dvc_deck::source::{self, Discovery};
use dvc_deck::status::{EXIT_STOPPED_BY_USER, Status, print_to_stdout};
use std::net::UdpSocket;
use std::sync::atomic::Ordering;

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
    eprintln!("sending to {}", cfg.server_addr);
    let report = print_to_stdout();
    report(Status::Target(cfg.server_addr.clone()));

    let mut publisher = Publisher::new(
        sock.try_clone()
            .unwrap_or_else(|e| die(EXIT_ENV, &format!("cannot clone UDP socket: {e}"))),
        Tuning {
            deadzone: cfg.deadzone,
            stick_noise: cfg.stick_noise,
        },
    );
    let imu_wanted = publisher.imu_wanted_flag();
    link::spawn_monitor(
        sock,
        timing::LINK_TIMEOUT,
        {
            let report = report.clone();
            move |reachable| {
                if reachable {
                    eprintln!("server reachable");
                } else {
                    eprintln!("no reply from the server (check server_addr and the PC's firewall)");
                }
                report(Status::Server(reachable));
            }
        },
        move |ack| {
            if imu_wanted.swap(ack.imu_wanted, Ordering::Relaxed) != ack.imu_wanted {
                eprintln!(
                    "server {} the motion sensors",
                    if ack.imu_wanted {
                        "wants"
                    } else {
                        "does not need"
                    }
                );
            }
        },
    );

    let (tx, rx) = std::sync::mpsc::channel();
    let discovery: Box<dyn Discovery> = match cfg.source {
        Source::Hidraw => Box::new(HidrawDiscovery),
        Source::Evdev => Box::new(EvdevDiscovery {
            device: cfg.device.clone(),
        }),
    };
    source::spawn(discovery, tx, report, timing::SOURCE_POLL);

    match run(&rx, &mut publisher, cfg.heartbeat()) {
        RunEnd::Stopped => {
            // This exit status tells the Deck app to close too.
            eprintln!("stop requested (View + Menu)");
            std::process::exit(EXIT_STOPPED_BY_USER);
        }
        RunEnd::SourcesGone => die(EXIT_ENV, "the controller readers stopped unexpectedly"),
    }
}
