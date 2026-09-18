//! The automation input source (ADR-0042): an agent takes and gives up
//! control through the same transaction that a device change takes, its
//! demand passes no stick mapping, and the first operator input wins.

use pilotage_input::content_digest;

use super::{DUALSENSE_ID, session, with_scheme};
use crate::coordinator::{AgentInput, AgentTick, ControlCoordinator};
use crate::plan::{AXIS_PITCH, AXIS_ROLL, AXIS_THROTTLE, AXIS_YAW, ControlPlan};
use crate::sample::{ButtonSample, DirectDemand, Mode, RawSample, SessionState};

const AGENT_ID: &str = "automation.intent-pilot/v1";
const ARM_BUTTON: usize = 9;
/// The gimbal reset button of the default scheme.
const RESET_BUTTON: usize = 11;

const DEMAND: DirectDemand = DirectDemand {
    roll: 0.25,
    pitch: -0.5,
    throttle: 0.5,
    yaw: 0.125,
};

fn flying(demand: DirectDemand) -> AgentInput {
    AgentInput {
        demand,
        arm: false,
        disarm: false,
    }
}

fn motion(plan: &ControlPlan) -> Option<[f32; 4]> {
    plan.motion.as_ref().map(|frame| {
        let value = |wanted: u16| {
            frame
                .axes()
                .iter()
                .find(|(axis, _)| *axis == wanted)
                .map_or(f32::NAN, |(_, value)| *value)
        };
        [
            value(AXIS_ROLL),
            value(AXIS_PITCH),
            value(AXIS_THROTTLE),
            value(AXIS_YAW),
        ]
    })
}

fn as_array(demand: DirectDemand) -> [f32; 4] {
    [demand.roll, demand.pitch, demand.throttle, demand.yaw]
}

/// One agent tick with no operator input: the sample, then the plan.
fn agent_tick(
    coordinator: &mut ControlCoordinator,
    input: AgentInput,
    state: &SessionState,
) -> (AgentTick, ControlPlan) {
    operator_tick(coordinator, &[], &[], input, state)
}

fn operator_tick(
    coordinator: &mut ControlCoordinator,
    pad_axes: &[f32],
    pad_pressed: &[usize],
    input: AgentInput,
    state: &SessionState,
) -> (AgentTick, ControlPlan) {
    let buttons: Vec<ButtonSample> = (0..16)
        .map(|index| ButtonSample {
            pressed: pad_pressed.contains(&index),
            value: if pad_pressed.contains(&index) {
                1.0
            } else {
                0.0
            },
        })
        .collect();
    let mut sample = RawSample::default();
    let tick = coordinator.agent_sample(pad_axes, &buttons, input, &mut sample);
    (tick, coordinator.evaluate(&sample, state))
}

/// Engages the agent and runs the handover to the first live tick.
fn engaged() -> ControlCoordinator {
    let mut coordinator = with_scheme();
    assert!(coordinator.engage_agent(AGENT_ID));
    let state = session(true, true);
    for _ in 0..3 {
        agent_tick(&mut coordinator, flying(DirectDemand::default()), &state);
    }
    coordinator
}

#[test]
fn an_agent_takes_control_through_the_neutral_handover() {
    let mut coordinator = with_scheme();
    let state = session(true, true);
    assert!(coordinator.engage_agent(AGENT_ID));
    assert!(coordinator.agent_engaged());

    let (tick, install) = agent_tick(&mut coordinator, flying(DEMAND), &state);
    assert_eq!(tick, AgentTick::Flown);
    assert_ne!(motion(&install), Some(as_array(DEMAND)));
    assert_eq!(coordinator.activation_revision(), 2, "the engage installs");
    assert_eq!(install.motion_lease, None, "the motion lease stays held");

    let (_, proof) = agent_tick(&mut coordinator, flying(DEMAND), &state);
    assert_eq!(
        motion(&proof),
        Some([0.0; 4]),
        "the first frame under the new revision is neutral"
    );
    let (_, live) = agent_tick(&mut coordinator, flying(DEMAND), &state);
    assert_eq!(motion(&live), Some(as_array(DEMAND)), "no stick shaping");
    assert_eq!(live.label, Some("AGENT: directive"));
}

#[test]
fn the_announcement_names_the_agent_while_it_is_the_source() {
    let mut coordinator = engaged();
    assert_eq!(coordinator.device_label(), AGENT_ID);
    assert_eq!(coordinator.device_revision(), 1);
    assert_eq!(
        coordinator.device_digest(),
        Some(content_digest(AGENT_ID.as_bytes()))
    );

    coordinator.disengage_agent();
    assert!(!coordinator.agent_engaged());
    let mut sample = RawSample::default();
    coordinator.key_sample(&mut sample);
    coordinator.evaluate(&sample, &session(true, true));
    assert_eq!(coordinator.device_label(), "Keyboard");
    assert_eq!(coordinator.activation_revision(), 3, "the release installs");
}

#[test]
fn an_operator_stick_disengages_the_agent_and_never_flies_the_deflection() {
    let mut coordinator = engaged();
    let state = session(true, true);
    let (_, live) = agent_tick(&mut coordinator, flying(DEMAND), &state);
    assert_eq!(motion(&live), Some(as_array(DEMAND)));

    let full_climb = [0.0, -1.0, 0.0, 0.0];
    let (tick, taken) = operator_tick(&mut coordinator, &full_climb, &[], flying(DEMAND), &state);
    assert_eq!(tick, AgentTick::OperatorOverride);
    assert!(!coordinator.agent_engaged());
    assert_eq!(motion(&taken), Some([0.0; 4]), "the handover emits neutral");

    let (tick, held) = operator_tick(&mut coordinator, &full_climb, &[], flying(DEMAND), &state);
    assert_eq!(tick, AgentTick::NotEngaged);
    assert_eq!(
        motion(&held),
        Some([0.0; 4]),
        "a held deflection cannot drive at the install"
    );
}

#[test]
fn an_operator_safety_press_disengages_the_agent_and_does_not_arm() {
    let mut coordinator = engaged();
    let state = session(true, true);
    let (tick, plan) = operator_tick(&mut coordinator, &[], &[ARM_BUTTON], flying(DEMAND), &state);
    assert_eq!(tick, AgentTick::OperatorOverride);
    assert!(!plan.arm, "the press that takes control does not also arm");
    // The press stays down through the install and the first live ticks:
    // its edge was consumed by the release, so it fires no arm later either.
    for _ in 0..4 {
        let (_, held) = operator_tick(&mut coordinator, &[], &[ARM_BUTTON], flying(DEMAND), &state);
        assert!(!held.arm, "a held press is not a new edge");
    }
    operator_tick(&mut coordinator, &[], &[], flying(DEMAND), &state);
    let (_, again) = operator_tick(&mut coordinator, &[], &[ARM_BUTTON], flying(DEMAND), &state);
    assert!(
        again.arm,
        "a release and a new press is the operator's own arm"
    );
}

#[test]
fn an_operator_key_disengages_the_agent() {
    let mut coordinator = engaged();
    let state = session(true, true);
    coordinator.key_event("w", true);
    let (tick, taken) = agent_tick(&mut coordinator, flying(DEMAND), &state);
    assert_eq!(tick, AgentTick::OperatorOverride);
    assert!(!coordinator.agent_engaged());
    assert_eq!(motion(&taken), Some([0.0; 4]), "the handover emits neutral");
    coordinator.key_event("w", false);
    for _ in 0..3 {
        agent_tick(&mut coordinator, flying(DEMAND), &state);
    }
    assert_eq!(coordinator.device_label(), "Keyboard");
}

#[test]
fn the_gimbal_reset_press_is_an_operator_input_too() {
    let mut coordinator = engaged();
    let state = session(true, true);
    let (tick, _) = operator_tick(
        &mut coordinator,
        &[],
        &[RESET_BUTTON],
        flying(DEMAND),
        &state,
    );
    assert_eq!(tick, AgentTick::OperatorOverride);
}

#[test]
fn a_new_session_starts_with_the_operator_as_the_source() {
    let mut coordinator = engaged();
    coordinator.begin_session();
    assert!(!coordinator.agent_engaged());
}

#[test]
fn an_agent_arm_request_fires_one_typed_edge() {
    let mut coordinator = engaged();
    let state = session(true, true);
    let arm = AgentInput {
        arm: true,
        ..flying(DirectDemand::default())
    };
    let (_, first) = agent_tick(&mut coordinator, arm, &state);
    assert!(first.arm, "the request is a typed arm");
    let (_, held) = agent_tick(&mut coordinator, arm, &state);
    assert!(!held.arm, "a held request is one edge");
    agent_tick(&mut coordinator, flying(DirectDemand::default()), &state);
    let (_, again) = agent_tick(&mut coordinator, arm, &state);
    assert!(again.arm, "a new request is a new edge");
}

#[test]
fn a_mode_with_no_velocity_law_flies_the_agent_neutral() {
    let mut coordinator = engaged();
    for mode in [Mode::Fpv, Mode::Rover] {
        let state = SessionState {
            mode,
            ..session(true, true)
        };
        let (_, plan) = agent_tick(&mut coordinator, flying(DEMAND), &state);
        assert_eq!(motion(&plan), Some([0.0; 4]), "{mode:?}");
    }
}

#[test]
fn a_demand_that_is_not_finite_reads_neutral_and_a_large_one_is_bounded() {
    let mut coordinator = engaged();
    let broken = DirectDemand {
        roll: f32::NAN,
        pitch: f32::INFINITY,
        throttle: 7.0,
        yaw: -7.0,
    };
    let (_, plan) = agent_tick(&mut coordinator, flying(broken), &session(true, true));
    assert_eq!(motion(&plan), Some([0.0, 0.0, 1.0, -1.0]));
}

#[test]
fn a_pad_change_keeps_the_agent_as_the_source() {
    let mut coordinator = engaged();
    let state = session(true, true);
    coordinator.select_device(DUALSENSE_ID);
    assert!(coordinator.agent_engaged());
    for _ in 0..3 {
        agent_tick(&mut coordinator, flying(DEMAND), &state);
    }
    assert_eq!(coordinator.device_label(), AGENT_ID);
    let (_, live) = agent_tick(&mut coordinator, flying(DEMAND), &state);
    assert_eq!(motion(&live), Some(as_array(DEMAND)));

    coordinator.deselect_device();
    assert!(coordinator.agent_engaged(), "a disconnect is not a release");
}

/// A pad that takes control back is the source that the announcement names.
/// Naming the keyboard there would attribute pad input to the keyboard profile.
#[test]
fn a_pad_that_takes_control_is_the_announced_source() {
    let mut coordinator = with_scheme();
    let state = session(true, true);
    coordinator.select_device(DUALSENSE_ID);
    operator_tick(
        &mut coordinator,
        &[0.0; 4],
        &[],
        flying(DirectDemand::default()),
        &state,
    );
    assert_eq!(coordinator.device_label(), "Sony DualSense");
    assert!(coordinator.engage_agent(AGENT_ID));
    for _ in 0..3 {
        agent_tick(&mut coordinator, flying(DEMAND), &state);
    }
    assert_eq!(coordinator.device_label(), AGENT_ID);

    let (tick, _) = operator_tick(
        &mut coordinator,
        &[0.0, -1.0, 0.0, 0.0],
        &[],
        flying(DEMAND),
        &state,
    );
    assert_eq!(tick, AgentTick::OperatorOverride);
    for _ in 0..3 {
        operator_tick(&mut coordinator, &[0.0; 4], &[], flying(DEMAND), &state);
    }
    assert_eq!(coordinator.device_label(), "Sony DualSense");
}

/// A pad that disconnects under the agent leaves the keyboard to return to.
#[test]
fn a_release_after_a_pad_disconnect_returns_to_the_keyboard() {
    let mut coordinator = with_scheme();
    let state = session(true, true);
    coordinator.select_device(DUALSENSE_ID);
    operator_tick(
        &mut coordinator,
        &[0.0; 4],
        &[],
        flying(DirectDemand::default()),
        &state,
    );
    assert!(coordinator.engage_agent(AGENT_ID));
    for _ in 0..3 {
        agent_tick(&mut coordinator, flying(DEMAND), &state);
    }
    coordinator.deselect_device();
    for _ in 0..3 {
        agent_tick(&mut coordinator, flying(DEMAND), &state);
    }
    coordinator.disengage_agent();
    for _ in 0..3 {
        agent_tick(&mut coordinator, flying(DEMAND), &state);
    }
    assert_eq!(coordinator.device_label(), "Keyboard");
}

/// An engaged agent keeps the identity that it announced. A second identity
/// with no handover would change the announcement under the same revision.
#[test]
fn a_second_identity_cannot_replace_an_engaged_agent() {
    let mut coordinator = engaged();
    let revision = coordinator.activation_revision();
    assert!(!coordinator.engage_agent("automation.other/v1"));
    assert_eq!(coordinator.device_label(), AGENT_ID);
    assert_eq!(coordinator.activation_revision(), revision);
    assert!(
        coordinator.engage_agent(AGENT_ID),
        "the same identity is accepted"
    );
}

#[test]
fn an_engage_needs_an_identity_and_an_active_scheme() {
    let mut coordinator = with_scheme();
    assert!(!coordinator.engage_agent("  "));
    assert!(!coordinator.agent_engaged());
    let mut bare = ControlCoordinator::new();
    assert!(!bare.engage_agent(AGENT_ID));
}

#[test]
fn a_tick_with_no_agent_engaged_is_the_operators() {
    let mut coordinator = with_scheme();
    let (tick, _) = agent_tick(&mut coordinator, flying(DEMAND), &session(true, true));
    assert_eq!(tick, AgentTick::NotEngaged);
}
