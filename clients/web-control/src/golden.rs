#![allow(clippy::expect_used, clippy::panic)]

//! Executes the SHARED golden-vector file (`../golden-vectors.json`) against
//! the native device stage + control runtime. The wasm suite
//! (`clients/web/control-runtime.test.mjs`) executes the SAME file through
//! the compiled artifact, so the two cannot drift: a native/wasm divergence
//! reddens exactly one of them.

use serde::Deserialize;
use std::collections::BTreeMap;

use crate::DEFAULT_PROFILE_BYTES;
use crate::coordinator::ControlCoordinator;
use crate::device::SelectOutcome;
use crate::plan::{AXIS_PITCH, AXIS_ROLL, AXIS_THROTTLE, AXIS_YAW, ControlPlan, LeaseAction};
use crate::sample::{ButtonSample, Mode, RawSample, SessionState};

const VECTORS: &str = include_str!("../golden-vectors.json");

/// Both harnesses build pad samples with this many buttons, so pressed-set
/// semantics match bit for bit.
const PAD_BUTTONS: usize = 16;

#[derive(Deserialize)]
struct Doc {
    #[allow(dead_code)]
    comment: String,
    groups: Vec<Group>,
}

#[derive(Deserialize)]
struct Group {
    name: String,
    steps: Vec<Step>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Step {
    select_device: Option<String>,
    add_device_profile: Option<AddDeviceProfile>,
    key_events: Vec<(String, bool)>,
    clear_keys: bool,
    pad: Option<Pad>,
    source: Option<String>,
    session: Option<Session>,
    expect: Option<Expect>,
    expect_bound: Option<BTreeMap<String, bool>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddDeviceProfile {
    layer: String,
    profile: serde_json::Value,
}

#[derive(Deserialize)]
struct Pad {
    axes: Vec<f32>,
    pressed: Vec<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    mode: String,
    #[serde(default)]
    gimbal_granted: bool,
    #[serde(default)]
    gimbal_denied: bool,
    #[serde(default = "default_true")]
    motion_granted: bool,
    #[serde(default)]
    motion_denied: bool,
    #[serde(default = "default_true")]
    motion_recovered: bool,
    #[serde(default = "default_generation")]
    generation: u32,
}

const fn default_true() -> bool {
    true
}
const fn default_generation() -> u32 {
    1
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Expect {
    gimbal_pitch: Option<f32>,
    gimbal_yaw: Option<f32>,
    gimbal_null: Option<bool>,
    recenter: Option<bool>,
    motion_roll: Option<f32>,
    motion_pitch: Option<f32>,
    motion_throttle: Option<f32>,
    motion_yaw: Option<f32>,
    motion_gated: Option<bool>,
    motion_live: Option<bool>,
    lease: Option<String>,
    capture: Option<bool>,
    arm: Option<bool>,
    disarm: Option<bool>,
    arm_suppressed: Option<bool>,
    disarm_suppressed: Option<bool>,
    select: Option<String>,
    label: Option<String>,
    motion_lease: Option<String>,
    activation_revision: Option<u32>,
    added: Option<bool>,
}

struct Harness {
    coordinator: ControlCoordinator,
}

impl Harness {
    fn fresh() -> Self {
        let mut coordinator = ControlCoordinator::new();
        let revision = coordinator.activate_scheme(DEFAULT_PROFILE_BYTES);
        assert_eq!(revision, 1, "default activates at rev 1");
        Self { coordinator }
    }

    /// Mirrors the wasm `select_device`: the TRANSACTIONAL path.
    fn select(&mut self, id: &str) -> SelectOutcome {
        self.coordinator.select_device(id)
    }

    fn tick(&mut self, step: &Step, session: &Session, ctx: &str) -> ControlPlan {
        let mut sample = RawSample::default();
        if let Some(pad) = &step.pad {
            let buttons: Vec<ButtonSample> = (0..PAD_BUTTONS)
                .map(|i| ButtonSample {
                    pressed: pad.pressed.contains(&i),
                    value: if pad.pressed.contains(&i) { 1.0 } else { 0.0 },
                })
                .collect();
            self.coordinator
                .pad_sample(&pad.axes, &buttons, &mut sample);
        } else {
            assert_eq!(step.source.as_deref(), Some("keys"), "{ctx}: tick source");
            self.coordinator.key_sample(&mut sample);
        }
        let state = SessionState {
            generation: session.generation,
            now_ms: 100_000.0,
            mode: Mode::from_str_or_pilot(&session.mode),
            connected: true,
            lease_granted: session.gimbal_granted,
            lease_denied: session.gimbal_denied,
            motion_granted: session.motion_granted,
            motion_denied: session.motion_denied,
            motion_recovered: session.motion_recovered,
        };
        self.coordinator.evaluate(&sample, &state)
    }
}

fn profile_layer(name: &str) -> pilotage_input::ProfileLayer {
    match name {
        "organization" => pilotage_input::ProfileLayer::Organization,
        "user" => pilotage_input::ProfileLayer::User,
        "vehicle" => pilotage_input::ProfileLayer::Vehicle,
        "session" => pilotage_input::ProfileLayer::Session,
        other => panic!("unknown profile layer {other}"),
    }
}

fn axis(frame: Option<&crate::plan::Frame>, id: u16) -> f32 {
    frame
        .and_then(|f| f.axes().iter().find(|(a, _)| *a == id).copied())
        .map_or(f32::NAN, |(_, v)| v)
}

fn check_motion(expect: &Expect, plan: &ControlPlan, ctx: &str) {
    let motion = plan.motion.as_ref();
    if expect.motion_gated == Some(true) {
        assert!(motion.is_none(), "{ctx}: motion must be gated");
    }
    if expect.motion_live == Some(true) {
        assert!(motion.is_some(), "{ctx}: motion must be live");
    }
    for (name, want, id) in [
        ("roll", expect.motion_roll, AXIS_ROLL),
        ("pitch", expect.motion_pitch, AXIS_PITCH),
        ("throttle", expect.motion_throttle, AXIS_THROTTLE),
        ("yaw", expect.motion_yaw, AXIS_YAW),
    ] {
        if let Some(want) = want {
            assert_eq!(axis(motion, id), want, "{ctx}: motion {name}");
        }
    }
}

fn check_gimbal(expect: &Expect, plan: &ControlPlan, ctx: &str) {
    let gimbal = plan.gimbal.as_ref();
    if expect.gimbal_null == Some(true) {
        assert!(gimbal.is_none(), "{ctx}: gimbal frame must be absent");
    }
    if let Some(want) = expect.gimbal_pitch {
        assert_eq!(axis(gimbal, AXIS_PITCH), want, "{ctx}: gimbal pitch");
    }
    if let Some(want) = expect.gimbal_yaw {
        assert_eq!(axis(gimbal, AXIS_YAW), want, "{ctx}: gimbal yaw");
    }
    if let Some(want) = expect.recenter {
        let fired = gimbal.is_some_and(|f| !f.edges().is_empty());
        assert_eq!(fired, want, "{ctx}: recenter edge");
    }
}

fn check_plan(expect: &Expect, plan: &ControlPlan, ctx: &str) {
    check_motion(expect, plan, ctx);
    check_gimbal(expect, plan, ctx);
    if let Some(want) = &expect.lease {
        let got = match plan.lease {
            Some(LeaseAction::Request) => "request",
            Some(LeaseAction::Release) => "release",
            None => "none",
        };
        assert_eq!(got, want, "{ctx}: lease action");
    }
    if let Some(want) = expect.capture {
        assert_eq!(plan.capture_active, want, "{ctx}: capture");
    }
    if let Some(want) = expect.arm {
        assert_eq!(plan.arm, want, "{ctx}: arm edge");
    }
    if let Some(want) = expect.disarm {
        assert_eq!(plan.disarm, want, "{ctx}: disarm edge");
    }
    if let Some(want) = expect.arm_suppressed {
        assert_eq!(plan.arm_suppressed, want, "{ctx}: arm suppressed");
    }
    if let Some(want) = expect.disarm_suppressed {
        assert_eq!(plan.disarm_suppressed, want, "{ctx}: disarm suppressed");
    }
    if let Some(want) = &expect.motion_lease {
        let got = match plan.motion_lease {
            Some(LeaseAction::Request) => "request",
            Some(LeaseAction::Release) => "release",
            None => "none",
        };
        assert_eq!(got, want, "{ctx}: motion lease action");
    }
}

fn run_step(harness: &mut Harness, step: &Step, ctx: &str) {
    let expect = step.expect.as_ref();
    if let Some(id) = &step.select_device {
        let got = match harness.select(id) {
            SelectOutcome::Refused => "refused",
            SelectOutcome::Exact => "exact",
            SelectOutcome::Fallback => "fallback",
        };
        if let Some(want) = expect.and_then(|e| e.select.as_ref()) {
            assert_eq!(got, want, "{ctx}: selection outcome");
        }
    }
    if let Some(add) = &step.add_device_profile {
        let bytes = serde_json::to_vec(&add.profile).expect("profile serializes");
        let added = harness
            .coordinator
            .add_device_profile(profile_layer(&add.layer), &bytes);
        if let Some(want) = expect.and_then(|e| e.added) {
            assert_eq!(added, want, "{ctx}: profile added");
        }
    }
    if let Some(bound) = &step.expect_bound {
        for (key, want) in bound {
            assert_eq!(
                harness.coordinator.stage().key_is_bound(key),
                *want,
                "{ctx}: key {key} bound"
            );
        }
    }
    for (key, pressed) in &step.key_events {
        harness.coordinator.key_event(key, *pressed);
    }
    if step.clear_keys {
        harness.coordinator.clear_keys();
    }
    if let Some(session) = &step.session {
        let plan = harness.tick(step, session, ctx);
        if let Some(expect) = expect {
            check_plan(expect, &plan, ctx);
        }
    }
    // Label and revision reflect the INSTALLED state, so they are checked
    // after any tick this step ran (a pending swap installs mid-tick).
    if let Some(want) = expect.and_then(|e| e.label.as_ref()) {
        assert_eq!(
            harness.coordinator.device_label(),
            want,
            "{ctx}: device label"
        );
    }
    if let Some(want) = expect.and_then(|e| e.activation_revision) {
        assert_eq!(
            harness.coordinator.activation_revision(),
            want,
            "{ctx}: activation revision"
        );
    }
}

#[test]
fn golden_vectors() {
    let doc: Doc = serde_json::from_str(VECTORS).expect("golden-vectors.json parses");
    assert!(!doc.groups.is_empty(), "vector file has groups");
    for group in &doc.groups {
        let mut harness = Harness::fresh();
        for (index, step) in group.steps.iter().enumerate() {
            let ctx = format!("{} / step {}", group.name, index + 1);
            run_step(&mut harness, step, &ctx);
        }
    }
}
