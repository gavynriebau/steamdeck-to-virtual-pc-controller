//! Keeping a Deck input source alive: find the devices that are present, open
//! the new ones, read each on its own thread, pick them up again when they come
//! back, and say whether data is arriving. The same policy serves the Deck
//! hardware (hidraw) and an evdev pad; only discovery and reading differ, and
//! they are the adapters plugged in here.

use crate::status::{Report, Status};
use dvc_common::controller::State;
use dvc_common::lock;
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Reads one open device.
pub trait DeviceReader: Send {
    /// Read states until the device goes away or `on_state` returns false.
    fn run(&mut self, on_state: &mut dyn FnMut(State) -> bool);
}

/// A device that is present but not yet open.
pub struct Candidate {
    /// Names the device across polls (for example its path).
    pub id: String,
    pub open: Box<dyn FnOnce() -> io::Result<Box<dyn DeviceReader>> + Send>,
}

/// Finds the devices that are present right now.
pub trait Discovery: Send {
    /// `active` holds the ids already being read, so an adapter can skip
    /// expensive searching when it has what it needs.
    fn present(&mut self, active: &HashSet<String>) -> Vec<Candidate>;
}

pub struct Supervisor {
    discovery: Box<dyn Discovery>,
    tx: Sender<State>,
    report: Report,
    active: Arc<Mutex<HashSet<String>>>,
    got_data: Arc<AtomicBool>,
    polls: u32,
    warned: bool,
}

impl Supervisor {
    pub fn new(discovery: Box<dyn Discovery>, tx: Sender<State>, report: Report) -> Self {
        Supervisor {
            discovery,
            tx,
            report,
            active: Arc::default(),
            got_data: Arc::default(),
            polls: 0,
            warned: false,
        }
    }

    /// One look at the world: start reading any new device, and say so if no
    /// data has arrived after a full interval.
    pub fn poll(&mut self) {
        self.polls += 1;
        let present = {
            let active = lock(&self.active).clone();
            self.discovery.present(&active)
        };
        for candidate in present {
            if lock(&self.active).contains(&candidate.id) {
                continue;
            }
            let mut reader = match (candidate.open)() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("cannot open {}: {e}", candidate.id);
                    continue;
                }
            };
            lock(&self.active).insert(candidate.id.clone());
            let (tx, report) = (self.tx.clone(), self.report.clone());
            let (active, got_data) = (self.active.clone(), self.got_data.clone());
            std::thread::spawn(move || {
                reader.run(&mut |state| {
                    if !got_data.swap(true, Ordering::Relaxed) {
                        eprintln!("controller data from {}", candidate.id);
                        report(Status::Controller(true));
                    }
                    tx.send(state).is_ok()
                });
                eprintln!("lost {}", candidate.id);
                let none_left = {
                    let mut active = lock(&active);
                    active.remove(&candidate.id);
                    active.is_empty()
                };
                // With the last device gone nothing may stay pressed.
                if none_left {
                    let _ = tx.send(State::default());
                }
            });
        }
        if self.polls >= 2 && !self.got_data.load(Ordering::Relaxed) && !self.warned {
            self.warned = true;
            eprintln!("no controller data yet (is the Deck controller available?)");
            (self.report)(Status::Controller(false));
        }
    }
}

/// Run a [`Supervisor`] in the background, polling every `interval`.
pub fn spawn(discovery: Box<dyn Discovery>, tx: Sender<State>, report: Report, interval: Duration) {
    std::thread::spawn(move || {
        let mut supervisor = Supervisor::new(discovery, tx, report);
        loop {
            supervisor.poll();
            std::thread::sleep(interval);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvc_common::controller::Button;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::{Receiver, channel};

    const WAIT: Duration = Duration::from_secs(2);

    /// A device that yields `states`, then waits for `finish` before ending.
    struct FakeReader {
        states: Vec<State>,
        finish: Receiver<()>,
    }

    impl DeviceReader for FakeReader {
        fn run(&mut self, on_state: &mut dyn FnMut(State) -> bool) {
            for s in self.states.drain(..) {
                if !on_state(s) {
                    return;
                }
            }
            let _ = self.finish.recv();
        }
    }

    /// What discovery reports; each present device can be given a way to end.
    #[derive(Clone, Default)]
    struct World {
        devices: Arc<Mutex<Vec<FakeDevice>>>,
        opens: Arc<AtomicUsize>,
        searches: Arc<AtomicUsize>,
    }

    #[derive(Clone)]
    struct FakeDevice {
        id: String,
        states: Vec<State>,
        /// Number of opens that fail before one succeeds.
        failures: Arc<AtomicUsize>,
        finish: Arc<Mutex<Option<std::sync::mpsc::Sender<()>>>>,
    }

    impl World {
        fn add(&self, id: &str, states: Vec<State>, failures: usize) -> FakeDevice {
            let d = FakeDevice {
                id: id.into(),
                states,
                failures: Arc::new(AtomicUsize::new(failures)),
                finish: Arc::default(),
            };
            lock(&self.devices).push(d.clone());
            d
        }
        fn remove(&self, id: &str) {
            lock(&self.devices).retain(|d| d.id != id);
        }
    }

    impl FakeDevice {
        /// Let the reader of the current open end (the device was unplugged).
        fn unplug(&self, world: &World) {
            world.remove(&self.id);
            if let Some(tx) = lock(&self.finish).take() {
                let _ = tx.send(());
            }
        }
    }

    impl Discovery for World {
        fn present(&mut self, _active: &HashSet<String>) -> Vec<Candidate> {
            self.searches.fetch_add(1, Ordering::SeqCst);
            lock(&self.devices)
                .iter()
                .map(|d| {
                    let (d, opens) = (d.clone(), self.opens.clone());
                    Candidate {
                        id: d.id.clone(),
                        open: Box::new(move || {
                            if d.failures.load(Ordering::SeqCst) > 0 {
                                d.failures.fetch_sub(1, Ordering::SeqCst);
                                return Err(io::Error::other("busy"));
                            }
                            opens.fetch_add(1, Ordering::SeqCst);
                            let (tx, rx) = channel();
                            *lock(&d.finish) = Some(tx);
                            Ok(Box::new(FakeReader {
                                states: d.states.clone(),
                                finish: rx,
                            }) as Box<dyn DeviceReader>)
                        }),
                    }
                })
                .collect()
        }
    }

    /// Reports collected in order.
    fn recorder() -> (Report, Arc<Mutex<Vec<Status>>>) {
        let log: Arc<Mutex<Vec<Status>>> = Arc::default();
        let sink = log.clone();
        (Arc::new(move |s| lock(&sink).push(s)), log)
    }

    fn pressed(b: Button) -> State {
        State::default().with(b)
    }

    fn supervisor(world: &World) -> (Supervisor, Receiver<State>, Arc<Mutex<Vec<Status>>>) {
        let (tx, rx) = channel();
        let (report, log) = recorder();
        (
            Supervisor::new(Box::new(world.clone()), tx, report),
            rx,
            log,
        )
    }

    #[test]
    fn a_new_device_is_opened_once_and_its_states_arrive() {
        let world = World::default();
        world.add("dev0", vec![pressed(Button::A)], 0);
        let (mut sup, rx, log) = supervisor(&world);
        sup.poll();
        assert!(rx.recv_timeout(WAIT).unwrap().pressed(Button::A));
        sup.poll();
        sup.poll();
        assert_eq!(
            world.opens.load(Ordering::SeqCst),
            1,
            "not reopened while active"
        );
        assert_eq!(*lock(&log), [Status::Controller(true)], "reported once");
    }

    #[test]
    fn an_unplugged_device_is_released_and_reopened_when_it_returns() {
        let world = World::default();
        let dev = world.add("dev0", vec![pressed(Button::B)], 0);
        let (mut sup, rx, _log) = supervisor(&world);
        sup.poll();
        assert!(rx.recv_timeout(WAIT).unwrap().pressed(Button::B));

        dev.unplug(&world);
        assert_eq!(
            rx.recv_timeout(WAIT).unwrap(),
            State::default(),
            "released on loss"
        );

        world.add("dev0", vec![pressed(Button::X)], 0); // plugged back in
        sup.poll();
        assert!(rx.recv_timeout(WAIT).unwrap().pressed(Button::X));
        assert_eq!(world.opens.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_device_that_fails_to_open_is_retried_on_the_next_poll() {
        let world = World::default();
        world.add("dev0", vec![pressed(Button::Y)], 1);
        let (mut sup, rx, _log) = supervisor(&world);
        sup.poll();
        assert!(rx.try_recv().is_err(), "nothing yet: the open failed");
        sup.poll();
        assert!(rx.recv_timeout(WAIT).unwrap().pressed(Button::Y));
    }

    #[test]
    fn one_of_several_devices_going_away_does_not_release_the_others() {
        // The Deck exposes several hidraw nodes; only one carries input.
        let world = World::default();
        let quiet = world.add("hidraw0", vec![], 0);
        world.add("hidraw2", vec![pressed(Button::A)], 0);
        let (mut sup, rx, _log) = supervisor(&world);
        sup.poll();
        assert!(rx.recv_timeout(WAIT).unwrap().pressed(Button::A));

        quiet.unplug(&world);
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            rx.try_recv().is_err(),
            "the working device keeps its buttons held"
        );
    }

    #[test]
    fn no_data_after_a_full_interval_is_reported_once() {
        let world = World::default();
        world.add("silent", vec![], 0);
        let (mut sup, _rx, log) = supervisor(&world);
        sup.poll();
        assert!(lock(&log).is_empty(), "too early to say");
        sup.poll();
        sup.poll();
        sup.poll();
        assert_eq!(*lock(&log), [Status::Controller(false)]);
    }

    #[test]
    fn no_device_at_all_is_reported_missing_too() {
        let (mut sup, _rx, log) = supervisor(&World::default());
        sup.poll();
        sup.poll();
        assert_eq!(*lock(&log), [Status::Controller(false)]);
    }

    #[test]
    fn data_arriving_cancels_the_missing_warning() {
        let world = World::default();
        world.add("dev0", vec![State::default()], 0);
        let (mut sup, rx, log) = supervisor(&world);
        sup.poll();
        rx.recv_timeout(WAIT).unwrap();
        sup.poll();
        sup.poll();
        assert_eq!(*lock(&log), [Status::Controller(true)]);
    }

    #[test]
    fn the_adapter_is_told_which_devices_are_active() {
        struct Spy(Arc<Mutex<Vec<usize>>>);
        impl Discovery for Spy {
            fn present(&mut self, active: &HashSet<String>) -> Vec<Candidate> {
                lock(&self.0).push(active.len());
                Vec::new()
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (tx, _rx) = channel();
        let (report, _) = recorder();
        let mut sup = Supervisor::new(Box::new(Spy(seen.clone())), tx, report);
        sup.poll();
        assert_eq!(*lock(&seen), [0]);
    }
}
