//! Runs on the PC: receives controller state over UDP and mirrors it onto a
//! virtual Xbox-style gamepad via uinput.

use evdev::uinput::VirtualDevice;
use evdev::{AbsInfo, AttributeSet, BusType, InputEvent, InputId, KeyCode, UinputAbsSetup, AbsoluteAxisCode};
use serde::Deserialize;
use std::net::UdpSocket;
use std::time::Duration;
use steamdeck_as_pc_controller::{config, proto, uhid};

const APP: &str = "deck-virtual-controller-server";
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;

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

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ControllerType {
    /// uinput Xbox 360 gamepad.
    Xbox,
    /// Emulated 2026 Steam Controller (28de:1302) over /dev/uhid: buttons, sticks, trackpads, gyro.
    Steam,
}

/// How packet fields map onto the virtual device's evdev codes.
struct Layout {
    id: InputId,
    /// evdev key code per `proto::BUTTONS` bit, `None` to drop that button.
    buttons: [Option<u16>; proto::BUTTONS.len()],
    /// evdev ABS code and range per `proto::AXES` entry.
    axes: [(u16, AbsInfo); 6],
    hat: [u16; 2],
}

impl Layout {
    /// Xbox 360 pad IDs so games/SDL apply their standard mapping.
    fn xbox() -> Layout {
        let stick = AbsInfo::new(0, proto::STICK_MIN, proto::STICK_MAX, 16, 128, 0);
        let trigger = AbsInfo::new(0, 0, proto::TRIGGER_MAX, 0, 0, 0);
        let mut buttons = [None; proto::BUTTONS.len()];
        for i in 0..proto::XBOX_BUTTONS {
            buttons[i] = Some(proto::BUTTONS[i]);
        }
        Layout {
            id: InputId::new(BusType::BUS_USB, 0x045e, 0x028e, 0x0114),
            buttons,
            axes: [
                (0x00, stick), (0x01, stick), (0x02, trigger),
                (0x03, stick), (0x04, stick), (0x05, trigger),
            ],
            hat: [proto::HAT_AXES[0], proto::HAT_AXES[1]],
        }
    }
}

fn build_device(name: &str, layout: &Layout) -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for code in layout.buttons.iter().flatten() {
        keys.insert(KeyCode(*code));
    }
    let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
    let mut b = VirtualDevice::builder()?.name(name).input_id(layout.id.clone()).with_keys(&keys)?;
    for &(code, info) in &layout.axes {
        b = b.with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode(code), info))?;
    }
    for &code in &layout.hat {
        b = b.with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode(code), hat))?;
    }
    b.build()
}

/// Events for everything that differs between `old` and `new`.
fn diff(layout: &Layout, old: &proto::State, new: &proto::State) -> Vec<InputEvent> {
    let mut ev = Vec::new();
    for (bit, code) in layout.buttons.iter().enumerate() {
        let (o, n) = (old.buttons >> bit & 1, new.buttons >> bit & 1);
        if let (true, Some(code)) = (o != n, code) {
            ev.push(InputEvent::new(EV_KEY, *code, n as i32));
        }
    }
    for i in 0..6 {
        if old.axes[i] != new.axes[i] {
            ev.push(InputEvent::new(EV_ABS, layout.axes[i].0, new.axes[i] as i32));
        }
    }
    for (i, &code) in layout.hat.iter().enumerate() {
        if old.hat[i] != new.hat[i] {
            ev.push(InputEvent::new(EV_ABS, code, new.hat[i] as i32));
        }
    }
    ev
}

fn main() {
    let cfg: Config = config::load(APP).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });

    let layout = Layout::xbox();
    // "steam" is a real (emulated) Steam Controller HID device; "xbox" is a uinput gamepad.
    let (mut pad, steam) = match cfg.controller_type {
        ControllerType::Xbox => (Some(build_device(&cfg.name, &layout)), None),
        ControllerType::Steam => (None, Some(uhid::SteamControllerDevice::create(&cfg.name))),
    };
    if let Some(Err(e)) = &pad {
        eprintln!("cannot create uinput device: {e} (need write access to /dev/uinput)");
        std::process::exit(1);
    }
    if let Some(Err(e)) = &steam {
        eprintln!("cannot create uhid device: {e} (need write access to /dev/uhid)");
        std::process::exit(1);
    }
    let mut pad = pad.take().map(Result::unwrap);
    // The Steam Controller device streams the latest state continuously.
    let shared_state = steam.map(|dev| {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(proto::State::default()));
        dev.unwrap().stream(shared.clone(), Duration::from_millis(4));
        shared
    });
    let sock = UdpSocket::bind(&cfg.bind_addr).unwrap_or_else(|e| {
        eprintln!("cannot bind {}: {e}", cfg.bind_addr);
        std::process::exit(1);
    });
    sock.set_read_timeout(Some(Duration::from_millis(cfg.timeout_ms.max(50)))).unwrap();
    eprintln!("listening on {}, virtual device '{}'", cfg.bind_addr, cfg.name);

    let mut current = proto::State::default();
    let mut last_seq: Option<u32> = None;
    let mut client = None;
    let mut buf = [0u8; 64];
    loop {
        let next = match sock.recv_from(&mut buf) {
            Ok((n, from)) => match proto::Packet::decode(&buf[..n]) {
                Some(p) if last_seq.is_none_or(|l| proto::is_newer(p.seq, l)) || client != Some(from) => {
                    if client != Some(from) {
                        eprintln!("client connected: {from}");
                        client = Some(from);
                    }
                    last_seq = Some(p.seq);
                    p.state
                }
                _ => continue,
            },
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if client.take().is_some() {
                    eprintln!("client timed out, releasing inputs");
                }
                last_seq = None;
                proto::State::default()
            }
            Err(e) => {
                eprintln!("recv error: {e}");
                continue;
            }
        };
        if let Some(shared) = &shared_state {
            *shared.lock().unwrap() = next;
            current = next;
            continue;
        }
        let events = diff(&layout, &current, &next);
        if !events.is_empty() {
            if let Err(e) = pad.as_mut().unwrap().emit(&events) {
                eprintln!("emit failed: {e}");
            }
            current = next;
        }
    }
}
