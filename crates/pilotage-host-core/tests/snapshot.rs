#![cfg(test)]

//! A copy of the core is its snapshot: the core against the traffic of
//! Aviate's C172 board (the `aviate_link` capture), with a first plan
//! loaded at 2 s and a second one at 10 s. A copy made during the flight
//! continues as the original does, and a core that goes back to a copy
//! repeats the bytes of its first pass.

use navigate_contract::{GeodeticPosition, MonotonicNanos};
use pilotage_adapter_api::SourceIncarnation;
use pilotage_adapter_aviate::{AviateAirframe, EmbeddedIncarnations};
use pilotage_control_feel::{FixedWingEnvelope, ValidatedFixedWingEnvelope};
use pilotage_host_core::automation::{AutomationStatus, MissionPlan, MissionTask, PlanSource};
use pilotage_host_core::engine_actor::ToEngine;
use pilotage_host_core::{HostCore, HostCoreConfig, HostEvent, HostStatus, MISSION_CLOCK};
use pilotage_mavlink::link::LinkConfig;
use pilotage_mission::{
    MissionConfig, MissionEngine, OwnshipSource, PlannedWaypoint, WaypointPlan,
};
use pilotage_protocol::VehicleClass;
use pilotage_session::SessionEngine;
use web_time::Instant;

const CAPTURE: &str = include_str!("fixtures/aviate-c172-link-15s.txt");
const LAT_DEG: f64 = 41.7;
const LON_DEG: f64 = -1.07;
const ALT_M: f64 = 2050.0;
const FIRST_AT_NS: u64 = 2_000_000_000;
const SWAP_AT_NS: u64 = 10_000_000_000;
/// The snapshot time of the tests that do not snapshot at the swap: the
/// first plan flies, and the second one is not loaded yet.
const SNAPSHOT_AT_NS: u64 = 6_000_000_000;
const END_NS: u64 = 15_000_000_000;

fn config() -> HostCoreConfig {
    let anchor = GeodeticPosition::new(LAT_DEG.to_radians(), LON_DEG.to_radians(), ALT_M);
    let mut mission = MissionConfig::new(String::new(), anchor, MISSION_CLOCK);
    mission.ownship_source = OwnshipSource::vehicle_estimate();
    mission.vehicle_class = VehicleClass::FixedWing;
    mission.cruise_height_m = 0.0;
    mission.cruise_mps = 55.0;
    mission.limits.max_horizontal_mps = 65.0;
    HostCoreConfig {
        link: LinkConfig::default(),
        airframe: AviateAirframe::FixedWing(
            ValidatedFixedWingEnvelope::new(FixedWingEnvelope::C172).unwrap(),
        ),
        fc_origin_height_m: Some(ALT_M),
        incarnations: EmbeddedIncarnations {
            estimate: SourceIncarnation::new([1; 16]),
            fc_state: SourceIncarnation::new([2; 16]),
        },
        mission,
        magnetic_epoch_unix_s: 1_790_000_000,
    }
}

fn first_plan() -> WaypointPlan {
    WaypointPlan::new(vec![
        PlannedWaypoint::new(LAT_DEG, LON_DEG + 0.03, ALT_M),
        PlannedWaypoint::new(LAT_DEG + 0.02, LON_DEG + 0.06, ALT_M),
    ])
}

fn second_plan() -> WaypointPlan {
    WaypointPlan::new(vec![
        PlannedWaypoint::new(LAT_DEG - 0.02, LON_DEG, ALT_M),
        PlannedWaypoint::new(LAT_DEG - 0.04, LON_DEG - 0.03, ALT_M),
    ])
}

/// One thing the embedding host gives the core.
#[derive(Clone)]
enum Input {
    /// A datagram from the flight controller, given to both links.
    Datagram(Vec<u8>),
    /// A plan to load.
    Load(WaypointPlan),
}

/// The inputs of the flight with their times, in the order the host gives
/// them: a plan before the datagrams of its time.
fn inputs() -> Vec<(u64, Input)> {
    let mut inputs = vec![
        (FIRST_AT_NS, Input::Load(first_plan())),
        (SWAP_AT_NS, Input::Load(second_plan())),
    ];
    for line in CAPTURE.lines() {
        let mut parts = line.split_whitespace();
        let (Some(at), Some("from_fc"), Some(text)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let bytes = (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect();
        inputs.push((at.parse().unwrap(), Input::Datagram(bytes)));
    }
    // A stable sort: the plans stay before the datagrams of their time.
    inputs.sort_by_key(|(at, _)| *at);
    inputs
}

/// What the core gave out, each with the time of its tick.
#[derive(Debug, Default, PartialEq)]
struct Trace {
    frames: Vec<(u64, Vec<u8>)>,
    events: Vec<(u64, HostEvent)>,
    statuses: Vec<(u64, HostStatus)>,
}

impl Trace {
    /// Fails at the first output that is not the same in `other`.
    #[track_caller]
    fn assert_same(&self, other: &Self) {
        for (index, (ours, theirs)) in self.frames.iter().zip(&other.frames).enumerate() {
            assert_eq!(ours, theirs, "uplink frame {index}");
        }
        assert_eq!(self.frames.len(), other.frames.len(), "uplink frames");
        for (index, (ours, theirs)) in self.events.iter().zip(&other.events).enumerate() {
            assert_eq!(ours, theirs, "event {index}");
        }
        assert_eq!(self.events.len(), other.events.len(), "events");
        for (ours, theirs) in self.statuses.iter().zip(&other.statuses) {
            assert_eq!(ours, theirs, "status");
        }
        assert_eq!(self.statuses.len(), other.statuses.len(), "statuses");
    }
}

/// A core in flight, with the host's own place in the inputs.
struct Run {
    core: HostCore,
    wake: MonotonicNanos,
    next_input: usize,
    trace: Trace,
}

impl Run {
    fn new() -> Self {
        Self {
            core: HostCore::new(config(), Instant::now()).unwrap(),
            wake: MonotonicNanos::from_nanos(0),
            next_input: 0,
            trace: Trace::default(),
        }
    }

    /// A second run from the same state, with an empty trace.
    fn snapshot(&self) -> Self {
        Self {
            core: self.core.try_clone().unwrap(),
            wake: self.wake,
            next_input: self.next_input,
            trace: Trace::default(),
        }
    }

    /// Runs every tick that is due until `until`.
    fn tick_to(&mut self, until: u64) {
        while self.wake.as_nanos() <= until {
            let at = self.wake.as_nanos();
            self.wake = self.core.tick(self.wake).unwrap().at;
            let frames = self.core.drain_uplink();
            self.trace
                .frames
                .extend(frames.into_iter().map(|frame| (at, frame)));
            let events = self.core.drain_events();
            self.trace
                .events
                .extend(events.into_iter().map(|event| (at, event)));
            self.trace.statuses.push((at, self.core.status().clone()));
        }
    }

    /// Gives the core its next input, after the ticks before it. Returns
    /// whether the input was a plan.
    fn step(&mut self, inputs: &[(u64, Input)]) -> bool {
        let (at, input) = inputs[self.next_input].clone();
        self.next_input += 1;
        self.tick_to(at);
        let now = MonotonicNanos::from_nanos(at);
        match input {
            Input::Datagram(datagram) => {
                self.core.on_downlink(&datagram, now);
                self.core.on_uplink_reply(&datagram, now);
                false
            }
            Input::Load(plan) => {
                self.core.load_mission(plan).unwrap();
                true
            }
        }
    }

    /// Gives the core every input until `until`, and runs to `until`.
    fn advance(&mut self, inputs: &[(u64, Input)], until: u64) {
        while inputs
            .get(self.next_input)
            .is_some_and(|(at, _)| *at <= until)
        {
            self.step(inputs);
        }
        self.tick_to(until);
    }

    /// Gives the core its inputs until the second plan is loaded, and
    /// stops there, before the next tick.
    fn advance_to_swap(&mut self, inputs: &[(u64, Input)]) {
        self.advance(inputs, SWAP_AT_NS - 1);
        assert!(self.step(inputs), "the next input is the second plan");
    }

    fn take_trace(&mut self) -> Trace {
        core::mem::take(&mut self.trace)
    }
}

/// The frames of a trace are a flight: the mission frames to its end.
fn assert_flies(trace: &Trace) {
    let last = trace.frames.last().map(|(at, _)| *at).unwrap_or_default();
    assert!(
        trace.frames.len() > 100 && last >= END_NS - 100_000_000,
        "{} frames, the last one at {last} ns",
        trace.frames.len()
    );
}

#[test]
fn a_copy_continues_as_the_original() {
    let inputs = inputs();
    let mut original = Run::new();
    original.advance(&inputs, SNAPSHOT_AT_NS);
    assert!(original.core.status().frames_sent > 0, "a mission flies");
    let mut copy = original.snapshot();
    original.take_trace();
    original.advance(&inputs, END_NS);
    copy.advance(&inputs, END_NS);
    let original_trace = original.take_trace();
    assert_flies(&original_trace);
    original_trace.assert_same(&copy.take_trace());
    assert_eq!(original.core.status(), copy.core.status());
    assert!(
        original_trace
            .events
            .iter()
            .any(|(_, event)| matches!(event, HostEvent::NewPlanFlying)),
        "the second plan took over after the snapshot"
    );
}

#[test]
fn a_core_that_goes_back_to_a_snapshot_repeats_its_first_pass() {
    let inputs = inputs();
    let mut run = Run::new();
    run.advance(&inputs, SNAPSHOT_AT_NS);
    let snapshot = run.snapshot();
    run.take_trace();
    run.advance(&inputs, END_NS);
    let first = run.take_trace();
    assert_flies(&first);
    // The snapshot stays as it was through each pass: a pass that wrote
    // into a part it shared with the snapshot would change the next one.
    for pass in 0..2 {
        run = snapshot.snapshot();
        run.advance(&inputs, END_NS);
        let again = run.take_trace();
        assert_eq!(first.frames.len(), again.frames.len(), "pass {pass}");
        first.assert_same(&again);
    }
}

#[test]
fn a_copy_holds_the_frames_and_events_not_drained() {
    let inputs = inputs();
    let mut original = Run::new();
    original.advance(&inputs, SNAPSHOT_AT_NS);
    // Ticks with their outputs left in the core, until a frame waits.
    let mut wake = original.wake;
    while original.core.try_clone().unwrap().drain_uplink().is_empty() {
        assert!(wake.as_nanos() < END_NS, "no frame until the end");
        wake = original.core.tick(wake).unwrap().at;
    }
    let mut copy = original.core.try_clone().unwrap();
    let frames = original.core.drain_uplink();
    assert!(!frames.is_empty());
    assert_eq!(frames, copy.drain_uplink());
    assert_eq!(original.core.drain_events(), copy.drain_events());
    assert!(copy.drain_uplink().is_empty(), "drained once");
}

#[test]
fn a_copy_made_while_a_new_plan_takes_over_continues_as_the_original() {
    let inputs = inputs();
    let mut original = Run::new();
    original.advance_to_swap(&inputs);
    let mut copy = original.snapshot();
    // The handover is not done at the snapshot: no event of it waits in
    // the core, and it comes in the flight after the snapshot.
    let waiting = original.core.try_clone().unwrap().drain_events();
    assert!(
        !waiting
            .iter()
            .any(|event| matches!(event, HostEvent::NewPlanFlying)),
        "{waiting:?}"
    );
    original.take_trace();
    original.advance(&inputs, END_NS);
    copy.advance(&inputs, END_NS);
    let original_trace = original.take_trace();
    assert_flies(&original_trace);
    original_trace.assert_same(&copy.take_trace());
    assert_eq!(original.core.status(), copy.core.status());
    assert!(
        original_trace
            .events
            .iter()
            .any(|(at, event)| *at >= SWAP_AT_NS && matches!(event, HostEvent::NewPlanFlying)),
        "the new plan took over in the flight after the snapshot"
    );
}

/// Fails to compile, with the name of the type, when a part of the core
/// stops being `Clone`.
fn assert_clone<T: Clone>() {}

#[test]
fn the_parts_of_the_core_are_clone() {
    assert_clone::<HostCoreConfig>();
    assert_clone::<HostStatus>();
    assert_clone::<HostEvent>();
    assert_clone::<MissionTask>();
    assert_clone::<MissionPlan>();
    assert_clone::<PlanSource>();
    assert_clone::<AutomationStatus>();
    assert_clone::<ToEngine>();
    assert_clone::<MissionEngine>();
    assert_clone::<SessionEngine>();
}
