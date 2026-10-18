//! Runs on the Steam Deck: reads the Deck's gamepad and streams its state over UDP.

use evdev::EventSummary;
use evdev::raw_stream::{RawDevice, enumerate as enumerate_raw};
use serde::Deserialize;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use steamdeck_as_pc_controller::config::Validate;
use steamdeck_as_pc_controller::controller::{self, axis};
use steamdeck_as_pc_controller::startup::{EXIT_CONFIG, EXIT_ENV, die, load_config};
use steamdeck_as_pc_controller::{deck_hid, lock, proto, xbox_pad};

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

/// Preference order when auto-detecting.
const PREFERRED: [&str; 2] = ["Microsoft X-Box 360 pad", "Valve Software Steam Controller"];

fn has_gamepad_buttons(d: &RawDevice) -> bool {
    d.supported_keys()
        .is_some_and(|k| k.contains(evdev::KeyCode(xbox_pad::BUTTONS[0].1)))
}

fn find_device(want: &str) -> Option<(std::path::PathBuf, RawDevice)> {
    let mut found: Vec<_> = enumerate_raw().collect();
    if !want.is_empty() {
        if want.starts_with('/') {
            return RawDevice::open(want).ok().map(|d| (want.into(), d));
        }
        return found.into_iter().find(|(_, d)| d.name() == Some(want));
    }
    found.retain(|(_, d)| has_gamepad_buttons(d));
    for pref in PREFERRED {
        if let Some(i) = found
            .iter()
            .position(|(_, d)| d.name().is_some_and(|n| n.starts_with(pref)))
        {
            return Some(found.swap_remove(i));
        }
    }
    None
}

struct Shared {
    state: controller::State,
    seq: u32,
}

fn send(sock: &UdpSocket, shared: &Mutex<Shared>, bump: bool) {
    let mut s = lock(shared);
    if bump {
        s.seq = s.seq.wrapping_add(1);
    }
    let pkt = proto::Packet {
        seq: s.seq,
        state: s.state,
    };
    let _ = sock.send(&pkt.encode());
}

/// View + Menu pressed together: tell the server to release everything, then exit.
/// The exit code 0 tells the Deck app to close too.
fn stop_streaming(sock: &UdpSocket, shared: &Mutex<Shared>) -> ! {
    lock(shared).state = controller::State::default();
    for _ in 0..3 {
        send(sock, shared, true);
        std::thread::sleep(Duration::from_millis(20));
    }
    eprintln!("stop requested (View + Menu)");
    std::process::exit(0);
}

fn apply(state: &mut controller::State, ev: evdev::InputEvent, deadzone: i32) -> bool {
    let before = *state;
    match ev.destructure() {
        EventSummary::Key(_, code, v) => {
            if let Some(button) = xbox_pad::button_for_code(code.0) {
                state.set(button, v != 0);
            }
        }
        EventSummary::AbsoluteAxis(_, code, v) => {
            if let Some(i) = xbox_pad::AXES.iter().position(|&c| c == code.0) {
                let v = if axis::STICKS.contains(&i) && v.abs() < deadzone {
                    0
                } else {
                    v
                };
                state.axes[i] = v.clamp(xbox_pad::STICK_MIN, xbox_pad::STICK_MAX) as i16;
            } else if let Some(i) = xbox_pad::HAT_AXES.iter().position(|&c| c == code.0) {
                state.hat[i] = v.clamp(-1, 1) as i8;
            }
        }
        _ => {}
    }
    *state != before
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

    let shared = Arc::new(Mutex::new(Shared {
        state: controller::State::default(),
        seq: 0,
    }));

    {
        let sock = sock
            .try_clone()
            .unwrap_or_else(|e| die(EXIT_ENV, &format!("cannot clone UDP socket: {e}")));
        let shared = shared.clone();
        let every = Duration::from_millis(cfg.heartbeat_ms.max(10));
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(every);
                send(&sock, &shared, false);
            }
        });
    }

    match cfg.source {
        Source::Hidraw => run_hidraw(&sock, &shared, cfg.deadzone),
        Source::Evdev => run_evdev(&sock, &shared, &cfg),
    }
}

fn run_hidraw(sock: &UdpSocket, shared: &Arc<Mutex<Shared>>, deadzone: i32) -> ! {
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};

    let got_data = Arc::new(AtomicBool::new(false));
    let mut warned = false;
    let mut started = Vec::new();
    loop {
        // The Deck exposes several hidraw nodes; only one emits state reports,
        // so read them all and ignore the rest.
        for path in deck_hid::find_nodes() {
            if started.contains(&path) {
                continue;
            }
            let Ok(mut file) = std::fs::File::open(&path) else {
                eprintln!("cannot open {}", path.display());
                continue;
            };
            let Ok(sock) = sock.try_clone() else { continue };
            started.push(path.clone());
            let shared = shared.clone();
            let got_data = got_data.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 128];
                while let Ok(n) = file.read(&mut buf) {
                    let Some(mut st) = deck_hid::parse_report(&buf[..n]) else {
                        continue;
                    };
                    if !got_data.swap(true, Ordering::Relaxed) {
                        eprintln!("controller data from {}", path.display());
                    }
                    for i in axis::STICKS {
                        if (st.axes[i] as i32).abs() < deadzone {
                            st.axes[i] = 0;
                        }
                    }
                    if st.is_stop_chord() {
                        stop_streaming(&sock, &shared);
                    }
                    let changed = {
                        let mut s = lock(&shared);
                        let changed = s.state != st;
                        s.state = st;
                        changed
                    };
                    if changed {
                        send(&sock, &shared, true);
                    }
                }
                eprintln!("lost {}", path.display());
            });
        }
        std::thread::sleep(Duration::from_secs(2));
        if !warned && !got_data.load(Ordering::Relaxed) {
            warned = true;
            eprintln!("no controller data yet (is the Deck controller available?)");
        }
    }
}

fn run_evdev(sock: &UdpSocket, shared: &Arc<Mutex<Shared>>, cfg: &Config) -> ! {
    loop {
        let Some((path, mut dev)) = find_device(&cfg.device) else {
            eprintln!("no gamepad device found, retrying");
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        eprintln!("reading {} ({})", path.display(), dev.name().unwrap_or("?"));
        loop {
            let events = match dev.fetch_events() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("device error: {e}");
                    break;
                }
            };
            let mut changed = false;
            for ev in events {
                changed |= apply(&mut lock(shared).state, ev, cfg.deadzone);
            }
            if lock(shared).state.is_stop_chord() {
                stop_streaming(sock, shared);
            }
            if changed {
                send(sock, shared, true);
            }
        }
        lock(shared).state = controller::State::default();
        send(sock, shared, true);
        std::thread::sleep(Duration::from_secs(1));
    }
}
