//! The real client wiring (`dvc_deck::run::run_client`) against the real server
//! wiring (`dvc_pc::run::run_server`) over loopback UDP, with a fake Deck as the
//! input source and a recording sink in place of the virtual device. Each test is
//! a situation that only shows up when both ends run together; two of them are
//! bugs that were found only by measuring a real Deck.

use dvc_common::controller::{Button, State};
use dvc_common::lock;
use dvc_deck::publisher::{RunEnd, Tuning};
use dvc_deck::run::{Settings, run_client};
use dvc_deck::source::{Candidate, DeviceReader, Discovery};
use dvc_deck::status::{Report, ServerLink, Status};
use dvc_pc::run::{Timing, run_server};
use dvc_pc::sink::{MemorySink, Sink};
use std::collections::HashSet;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// Fast timings that keep the protocol's required ordering:
// heartbeat x2 <= silence timeout, max(heartbeat, ack) x2 <= link timeout.
const HEARTBEAT: Duration = Duration::from_millis(20);
const SILENCE_TIMEOUT: Duration = Duration::from_millis(300);
const ACK_INTERVAL: Duration = Duration::from_millis(40);
const LINK_TIMEOUT: Duration = Duration::from_millis(400);
const REPORT_EVERY: Duration = Duration::from_millis(2); // like the Deck's ~4 ms

/// A fake Deck: emits whatever the test has set, every couple of milliseconds,
/// like the real controller delivers a report whether or not anything changed.
#[derive(Clone, Default)]
struct FakeDeck {
    state: Arc<Mutex<State>>,
    /// Add motion-sensor and stick jitter to every report.
    noise: Arc<AtomicBool>,
    ticks: Arc<AtomicU32>,
}

impl FakeDeck {
    fn set(&self, f: impl FnOnce(&mut State)) {
        f(&mut lock(&self.state));
    }
}

struct FakeReader(FakeDeck);

impl DeviceReader for FakeReader {
    fn run(&mut self, on_state: &mut dyn FnMut(State) -> bool) {
        loop {
            let mut state = *lock(&self.0.state);
            if self.0.noise.load(Ordering::Relaxed) {
                let n = self.0.ticks.fetch_add(1, Ordering::Relaxed) as i16;
                state.imu.accel[0] = n;
                state.imu.gyro[1] = n.wrapping_mul(3);
            }
            if !on_state(state) {
                return;
            }
            thread::sleep(REPORT_EVERY);
        }
    }
}

impl Discovery for FakeDeck {
    fn present(&mut self, active: &HashSet<String>) -> Vec<Candidate> {
        if !active.is_empty() {
            return Vec::new();
        }
        let deck = self.clone();
        vec![Candidate {
            id: "fake-deck".into(),
            open: Box::new(move || Ok(Box::new(FakeReader(deck)) as Box<dyn DeviceReader>)),
        }]
    }
}

/// A recording sink the test can look at while the server runs.
#[derive(Clone)]
struct SharedSink(Arc<Mutex<MemorySink>>);

impl Sink for SharedSink {
    fn apply(&mut self, state: &State) {
        lock(&self.0).apply(state);
    }
    fn wants_imu(&self) -> bool {
        lock(&self.0).wants_imu()
    }
}

fn settings() -> Settings {
    Settings {
        target: "loopback".into(),
        tuning: Tuning {
            deadzone: 0,
            stick_noise: 0,
        },
        heartbeat: HEARTBEAT,
        link_timeout: LINK_TIMEOUT,
        source_poll: Duration::from_millis(20),
    }
}

fn recorder() -> (Report, Arc<Mutex<Vec<Status>>>) {
    let log: Arc<Mutex<Vec<Status>>> = Arc::default();
    let sink = log.clone();
    (Arc::new(move |s| lock(&sink).push(s)), log)
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// A running Deck client talking to a running server.
struct Rig {
    deck: FakeDeck,
    sink: SharedSink,
    statuses: Arc<Mutex<Vec<Status>>>,
    client: Option<JoinHandle<RunEnd>>,
    server: Option<JoinHandle<()>>,
    stop_server: Arc<AtomicBool>,
}

impl Rig {
    fn start(imu_wanted: bool) -> Rig {
        let server_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server_sock.local_addr().unwrap();
        let sink = SharedSink(Arc::new(Mutex::new(MemorySink::default())));
        lock(&sink.0).imu_wanted = imu_wanted;
        let stop_server = Arc::new(AtomicBool::new(false));
        let server = {
            let (mut sink, stop) = (sink.clone(), stop_server.clone());
            thread::spawn(move || {
                let timing = Timing {
                    silence_timeout: SILENCE_TIMEOUT,
                    ack_interval: ACK_INTERVAL,
                };
                run_server(&server_sock, &mut sink, &timing, &stop).unwrap();
            })
        };

        let client_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        client_sock.connect(addr).unwrap();
        let deck = FakeDeck::default();
        let (report, statuses) = recorder();
        let client = {
            let deck = deck.clone();
            thread::spawn(move || {
                run_client(client_sock, Box::new(deck), report, &settings()).unwrap()
            })
        };
        Rig {
            deck,
            sink,
            statuses,
            client: Some(client),
            server: Some(server),
            stop_server,
        }
    }

    fn applied(&self) -> Vec<State> {
        lock(&self.sink.0).applied.clone()
    }

    fn statuses(&self) -> Vec<Status> {
        lock(&self.statuses).clone()
    }

    /// End the client the way a user does: View + Menu.
    fn press_the_stop_chord(&mut self) -> RunEnd {
        self.deck.set(|s| {
            s.set(Button::View, true);
            s.set(Button::Menu, true);
        });
        self.client.take().unwrap().join().unwrap()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop_server.store(true, Ordering::Relaxed);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

#[test]
fn a_button_pressed_on_the_deck_reaches_the_sink_and_its_release_follows() {
    let rig = Rig::start(true);
    rig.deck.set(|s| s.set(Button::A, true));
    wait_until("A to arrive", || {
        rig.applied().last().is_some_and(|s| s.pressed(Button::A))
    });
    rig.deck.set(|s| s.set(Button::A, false));
    wait_until("A to be released", || {
        rig.applied().last().is_some_and(|s| !s.pressed(Button::A))
    });
}

#[test]
fn a_held_button_survives_steady_unchanged_reports() {
    // Regression: heartbeats used to fire only when reports stopped, but the Deck
    // delivers one every few milliseconds even at rest, so a held, unchanging
    // state sent no heartbeat and the server released it after its silence timeout.
    let rig = Rig::start(true);
    rig.deck.set(|s| s.set(Button::A, true));
    wait_until("A to arrive", || {
        rig.applied().last().is_some_and(|s| s.pressed(Button::A))
    });
    let applied_when_pressed = rig.applied().len();

    thread::sleep(SILENCE_TIMEOUT * 4); // far longer than the server tolerates silence
    let after = rig.applied();
    assert!(
        after[applied_when_pressed..]
            .iter()
            .all(|s| s.pressed(Button::A)),
        "the server released a button that was still held: {} states applied since",
        after.len() - applied_when_pressed
    );
    assert!(after.last().unwrap().pressed(Button::A));
}

#[test]
fn a_quiet_deck_is_still_acked_so_the_link_stays_reachable() {
    // Regression: the server used to ack only packets it accepted, and treated a
    // heartbeat (a repeat of the last sequence number) as stale, so an idle Deck
    // got no acks and showed 'no reply'.
    let rig = Rig::start(true);
    wait_until("the link to come up", || {
        rig.statuses()
            .contains(&Status::Server(ServerLink::Reachable))
    });
    thread::sleep(LINK_TIMEOUT * 4); // nothing changes the whole time
    let statuses = rig.statuses();
    assert!(
        !statuses.contains(&Status::Server(ServerLink::Unreachable)),
        "an idle but connected Deck was reported unreachable: {statuses:?}"
    );
}

#[test]
fn motion_sensor_noise_is_sent_while_the_server_wants_it() {
    let rig = Rig::start(true);
    rig.deck.noise.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(200));
    let before = rig.applied().len();
    thread::sleep(Duration::from_millis(500));
    let sent = rig.applied().len() - before;
    assert!(
        sent > 50,
        "only {sent} states in 500 ms of changing sensor data"
    );
}

#[test]
fn motion_sensor_noise_stops_being_traffic_once_the_server_says_it_does_not_want_it() {
    let rig = Rig::start(false); // like the Xbox pad: no use for motion data
    rig.deck.noise.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(400)); // the first acks tell the Deck
    let before = rig.applied().len();
    thread::sleep(Duration::from_millis(500));
    let sent = rig.applied().len() - before;
    assert!(
        sent <= 3,
        "{sent} states in 500 ms although the server does not want motion data"
    );
}

#[test]
fn the_stop_chord_ends_the_client_and_the_server_releases_the_controller() {
    let mut rig = Rig::start(true);
    rig.deck.set(|s| s.set(Button::B, true));
    wait_until("B to arrive", || {
        rig.applied().last().is_some_and(|s| s.pressed(Button::B))
    });

    assert_eq!(rig.press_the_stop_chord(), RunEnd::Stopped);
    wait_until("the server to release everything", || {
        rig.applied().last() == Some(&State::default())
    });
}

#[test]
fn a_server_that_never_answers_is_reported_unreachable() {
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap(); // bound, but nobody reads it
    let client_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    client_sock.connect(silent.local_addr().unwrap()).unwrap();
    let (report, statuses) = recorder();
    let deck = FakeDeck::default();
    let chord = deck.clone();
    let client = thread::spawn(move || {
        run_client(client_sock, Box::new(deck), report, &settings()).unwrap()
    });
    wait_until("the unreachable report", || {
        lock(&statuses).contains(&Status::Server(ServerLink::Unreachable))
    });
    assert!(!lock(&statuses).contains(&Status::Server(ServerLink::Reachable)));
    chord.set(|s| {
        s.set(Button::View, true);
        s.set(Button::Menu, true);
    });
    assert_eq!(client.join().unwrap(), RunEnd::Stopped);
}

#[test]
fn a_server_from_the_previous_protocol_version_is_reported_incompatible_not_unreachable() {
    // Stand-in for an un-updated PC: it answers every packet with a version-2 ack.
    let old_server = UdpSocket::bind("127.0.0.1:0").unwrap();
    old_server
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let addr = old_server.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let responder = {
        let stop = stop.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 64];
            while !stop.load(Ordering::Relaxed) {
                if let Ok((_, from)) = old_server.recv_from(&mut buf) {
                    let _ = old_server.send_to(b"DVA2\x01\0\0\0", from);
                }
            }
        })
    };

    let client_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    client_sock.connect(addr).unwrap();
    let (report, statuses) = recorder();
    let deck = FakeDeck::default();
    let chord = deck.clone();
    let client = thread::spawn(move || {
        run_client(client_sock, Box::new(deck), report, &settings()).unwrap()
    });
    wait_until("the incompatible report", || {
        lock(&statuses).contains(&Status::Server(ServerLink::Incompatible))
    });
    thread::sleep(LINK_TIMEOUT * 2);
    assert!(
        !lock(&statuses).contains(&Status::Server(ServerLink::Unreachable)),
        "an answering server must not be called unreachable"
    );
    chord.set(|s| {
        s.set(Button::View, true);
        s.set(Button::Menu, true);
    });
    assert_eq!(client.join().unwrap(), RunEnd::Stopped);
    stop.store(true, Ordering::Relaxed);
    responder.join().unwrap();
}

#[test]
fn the_client_tells_the_deck_app_where_it_sends() {
    let mut rig = Rig::start(true);
    wait_until("the target report", || {
        rig.statuses().contains(&Status::Target("loopback".into()))
    });
    wait_until("the controller report", || {
        rig.statuses().contains(&Status::Controller(true))
    });
    assert_eq!(rig.press_the_stop_chord(), RunEnd::Stopped);
}
