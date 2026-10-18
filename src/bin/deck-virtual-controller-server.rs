//! Runs on the PC: receives controller state over UDP and mirrors it onto a
//! virtual Xbox-style gamepad via uinput.

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, InputEvent, InputId, KeyCode, UinputAbsSetup,
};
use serde::Deserialize;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use steamdeck_as_pc_controller::config::Validate;
use steamdeck_as_pc_controller::controller::{self, Button};
use steamdeck_as_pc_controller::proto::{self, DecodeError};
use steamdeck_as_pc_controller::session::{Accept, Session};
use steamdeck_as_pc_controller::startup::{EXIT_ENV, die, load_config};
use steamdeck_as_pc_controller::{lock, uhid, xbox_pad};

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

impl Validate for Config {}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ControllerType {
    /// uinput Xbox 360 gamepad.
    Xbox,
    /// Emulated 2026 Steam Controller (28de:1302) over /dev/uhid: buttons, sticks, trackpads, gyro.
    Steam,
}

/// How controller state maps onto the virtual device's evdev codes.
struct Layout {
    id: InputId,
    /// Button and its evdev key code.
    buttons: Vec<(Button, u16)>,
    /// evdev ABS code and range per `State::axes` entry.
    axes: [(u16, AbsInfo); 6],
    hat: [u16; 2],
}

impl Layout {
    /// Xbox 360 pad IDs so games/SDL apply their standard mapping.
    fn xbox() -> Layout {
        let stick = AbsInfo::new(0, xbox_pad::STICK_MIN, xbox_pad::STICK_MAX, 16, 128, 0);
        let trigger = AbsInfo::new(0, 0, controller::TRIGGER_MAX as i32, 0, 0, 0);
        let [lx, ly, lt, rx, ry, rt] = xbox_pad::AXES;
        Layout {
            id: InputId::new(BusType::BUS_USB, 0x045e, 0x028e, 0x0114),
            buttons: xbox_pad::BUTTONS[..xbox_pad::XBOX_BUTTON_COUNT].to_vec(),
            axes: [
                (lx, stick),
                (ly, stick),
                (lt, trigger),
                (rx, stick),
                (ry, stick),
                (rt, trigger),
            ],
            hat: xbox_pad::HAT_AXES,
        }
    }
}

fn build_device(name: &str, layout: &Layout) -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for &(_, code) in &layout.buttons {
        keys.insert(KeyCode(code));
    }
    let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
    let mut b = VirtualDevice::builder()?
        .name(name)
        .input_id(layout.id.clone())
        .with_keys(&keys)?;
    for &(code, info) in &layout.axes {
        b = b.with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode(code), info))?;
    }
    for &code in &layout.hat {
        b = b.with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode(code), hat))?;
    }
    b.build()
}

/// Events for everything that differs between `old` and `new`.
fn diff(layout: &Layout, old: &controller::State, new: &controller::State) -> Vec<InputEvent> {
    let mut ev = Vec::new();
    for &(button, code) in &layout.buttons {
        let down = new.pressed(button);
        if old.pressed(button) != down {
            ev.push(InputEvent::new(EV_KEY, code, down as i32));
        }
    }
    for i in 0..6 {
        if old.axes[i] != new.axes[i] {
            ev.push(InputEvent::new(
                EV_ABS,
                layout.axes[i].0,
                new.axes[i] as i32,
            ));
        }
    }
    for (i, &code) in layout.hat.iter().enumerate() {
        if old.hat[i] != new.hat[i] {
            ev.push(InputEvent::new(EV_ABS, code, new.hat[i] as i32));
        }
    }
    ev
}

/// Where decoded controller state goes.
enum Output {
    /// uinput Xbox pad: events are emitted when the state changes.
    Uinput(Box<(VirtualDevice, Layout)>),
    /// Emulated Steam Controller: a streamer thread sends the latest state.
    Hid(Arc<Mutex<controller::State>>),
}

fn fail(msg: &str) -> ! {
    die(EXIT_ENV, msg)
}

fn open_output(cfg: &Config) -> Output {
    match cfg.controller_type {
        ControllerType::Xbox => {
            let layout = Layout::xbox();
            let dev = build_device(&cfg.name, &layout).unwrap_or_else(|e| {
                fail(&format!(
                    "cannot create uinput device: {e} (need write access to /dev/uinput)"
                ))
            });
            Output::Uinput(Box::new((dev, layout)))
        }
        ControllerType::Steam => {
            let dev = uhid::SteamControllerDevice::create(&cfg.name).unwrap_or_else(|e| {
                fail(&format!(
                    "cannot create uhid device: {e} (need write access to /dev/uhid)"
                ))
            });
            let shared = Arc::new(Mutex::new(controller::State::default()));
            dev.stream(shared.clone(), Duration::from_millis(4));
            Output::Hid(shared)
        }
    }
}

fn describe(reason: DecodeError) -> String {
    match reason {
        DecodeError::NotOurs => "not a deck-virtual-controller packet".into(),
        DecodeError::Version(v) => format!(
            "protocol version {} (this server speaks {}); update the Deck client",
            v as char,
            proto::MAGIC[3] as char
        ),
        DecodeError::Length(n) => format!("wrong packet size ({n} bytes)"),
    }
}

fn main() {
    let cfg: Config = load_config(APP);

    let mut output = open_output(&cfg);
    let sock = UdpSocket::bind(&cfg.bind_addr)
        .unwrap_or_else(|e| fail(&format!("cannot bind {}: {e}", cfg.bind_addr)));
    sock.set_read_timeout(Some(Duration::from_millis(cfg.timeout_ms.max(50))))
        .unwrap_or_else(|e| fail(&format!("cannot set socket timeout: {e}")));
    eprintln!(
        "listening on {}, virtual device '{}'",
        cfg.bind_addr, cfg.name
    );

    let mut current = controller::State::default();
    let mut session = Session::new();
    let mut buf = [0u8; 64];
    loop {
        let next = match sock.recv_from(&mut buf) {
            Ok((n, from)) => match session.accept(from, &buf[..n]) {
                Accept::State { state, new_client } => {
                    if new_client {
                        eprintln!("client connected: {from}");
                    }
                    state
                }
                Accept::Stale => continue,
                Accept::Rejected { reason, first } => {
                    if first {
                        eprintln!("ignoring packets from {from}: {}", describe(reason));
                    }
                    continue;
                }
            },
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if session.on_silence() {
                    eprintln!("client timed out, releasing inputs");
                }
                controller::State::default()
            }
            Err(e) => {
                eprintln!("recv error: {e}");
                continue;
            }
        };
        match &mut output {
            Output::Hid(shared) => {
                *lock(shared) = next;
                current = next;
            }
            Output::Uinput(uinput) => {
                let (dev, layout) = &mut **uinput;
                let events = diff(layout, &current, &next);
                if !events.is_empty() {
                    if let Err(e) = dev.emit(&events) {
                        eprintln!("emit failed: {e}");
                    }
                    current = next;
                }
            }
        }
    }
}
