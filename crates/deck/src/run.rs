//! The client's wiring: the UDP socket, the server-link monitor, the input
//! source and the publisher, joined into one running client. The binary only
//! reads config and picks the source; everything that decides how the pieces
//! work together lives here, so it can be run against a fake Deck and a real
//! server in tests.

use crate::config::Config;
use crate::link;
use crate::publisher::{Publisher, RunEnd, Tuning, run};
use crate::source::{self, Discovery};
use crate::status::{Report, ServerLink, Status};
use dvc_common::{info, proto, timing, warn};
use std::io;
use std::net::UdpSocket;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Everything about a client run that is not the socket or the input source.
pub struct Settings {
    /// What to tell the Deck app we are sending to (shown on its screen).
    pub target: String,
    pub tuning: Tuning,
    pub heartbeat: Duration,
    pub link_timeout: Duration,
    pub source_poll: Duration,
}

impl Settings {
    /// The settings a user's config asks for, with the protocol's own timings.
    pub fn from_config(cfg: &Config) -> Self {
        Settings {
            target: cfg.server_addr.clone(),
            tuning: Tuning {
                deadzone: cfg.deadzone,
                stick_noise: cfg.stick_noise,
            },
            heartbeat: cfg.heartbeat(),
            link_timeout: timing::LINK_TIMEOUT,
            source_poll: timing::SOURCE_POLL,
        }
    }
}

/// Run the client until the user asks to stop (View + Menu) or the input
/// source disappears for good. `sock` must be connected to the server.
pub fn run_client(
    sock: UdpSocket,
    discovery: Box<dyn Discovery>,
    report: Report,
    settings: &Settings,
) -> io::Result<RunEnd> {
    info!("sending to {}", settings.target);
    report(Status::Target(settings.target.clone()));

    let mut publisher = Publisher::new(sock.try_clone()?, settings.tuning);
    let imu_wanted = publisher.imu_wanted_flag();
    link::spawn_monitor(
        sock,
        settings.link_timeout,
        {
            let report = report.clone();
            move |link| {
                match link {
                    ServerLink::Reachable => info!("server reachable"),
                    ServerLink::Unreachable => {
                        warn!("no reply from the server (check server_addr and the PC's firewall)")
                    }
                    ServerLink::Incompatible => warn!(
                        "the server speaks a different protocol version (this Deck speaks {}); update the PC server to match",
                        char::from(proto::VERSION)
                    ),
                }
                report(Status::Server(link));
            }
        },
        move |ack| {
            if imu_wanted.swap(ack.imu_wanted, Ordering::Relaxed) != ack.imu_wanted {
                info!(
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
    source::spawn(discovery, tx, report, settings.source_poll);
    Ok(run(&rx, &mut publisher, settings.heartbeat))
}
