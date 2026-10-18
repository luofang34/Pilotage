#![allow(clippy::expect_used, clippy::panic)]

//! Adapter-boundary behavior over a test-owned link: gating, arm refusal
//! with Betaflight's reasons, the demand hand-off, link-loss latching,
//! and the telemetry sample shape.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason, VehicleAdapter};
use pilotage_msp::{ArmingBlocker, ArmingDisableFlags};
use pilotage_protocol::{
    ControlAction, ControlIntent, ControlPayload, Generation, IntentFamily, ReferenceFrame,
    ScopeId, ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;
use tokio::sync::watch;

use super::{BetaflightAdapter, FLIGHT_SCOPE};
use crate::link::{ConfigIssue, LinkCommand, LinkReport, Observation};
use crate::rc::Sticks;
use crate::snapshot::tests::sample_snapshot;

fn frame(velocity: Option<[f32; 4]>, actions: Vec<ControlAction>) -> ScopedControlFrame {
    ScopedControlFrame {
        action_ids: vec![],
        session: SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new(FLIGHT_SCOPE),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent: velocity.map(|[vx, vy, vz, yaw_rate]| {
            ControlIntent::Velocity(VelocityIntent {
                frame: ReferenceFrame::BodyFrd,
                vx,
                vy,
                vz,
                yaw_rate,
            })
        }),
        actions,
    }
}

fn observation(armed: bool, blockers: u32, fix: bool) -> Observation {
    let mut snapshot = sample_snapshot();
    snapshot.armed = armed;
    snapshot.arming_disable = ArmingDisableFlags(blockers);
    if !fix {
        snapshot.gnss = None;
    }
    Observation {
        snapshot,
        north_east: fix.then_some([4.0, -2.0]),
        anchor: fix.then_some([47.397_742, 8.545_594]),
        flying: armed,
        hover_throttle: 0.55,
        sticks: Sticks::IDLE,
    }
}

struct Harness {
    adapter: BetaflightAdapter,
    reports: watch::Sender<LinkReport>,
    commands: watch::Receiver<LinkCommand>,
}

fn harness(latest: Option<Observation>) -> Harness {
    let (adapter, reports, commands) = BetaflightAdapter::detached(VehicleId::new(1));
    reports.send_modify(|report| report.latest = latest);
    Harness {
        adapter,
        reports,
        commands,
    }
}

#[test]
fn the_advertisement_is_one_velocity_scope_with_arm_and_disarm() {
    let h = harness(None);
    let capabilities = h.adapter.capabilities();
    let vehicle = &capabilities.vehicles[0];
    assert_eq!(vehicle.class, pilotage_protocol::VehicleClass::Multicopter);
    assert_eq!(vehicle.display_name, "Betaflight quad");
    assert_eq!(vehicle.scopes.len(), 2);
    let direct = &vehicle.scopes[1];
    assert_eq!(direct.scope.as_str(), super::DIRECT_SCOPE);
    assert_eq!(direct.authority_group, vehicle.scopes[0].authority_group);
    assert!(direct.authority_group.is_some());
    let families: Vec<IntentFamily> = direct.intents.iter().map(|i| i.family).collect();
    assert_eq!(
        families,
        vec![IntentFamily::BodyRate, IntentFamily::AttitudeThrust]
    );
    assert!((direct.intents[0].max_angular - 670f32.to_radians()).abs() < 1e-4);
    assert!((direct.intents[1].max_angular - 30f32.to_radians()).abs() < 1e-6);
    assert!(direct.legacy.is_none());
    let scope = &vehicle.scopes[0];
    assert_eq!(scope.scope.as_str(), FLIGHT_SCOPE);
    let intent = &scope.intents[0];
    assert_eq!(intent.family, IntentFamily::Velocity);
    assert_eq!(intent.frames, vec![ReferenceFrame::BodyFrd]);
    assert_eq!(
        (intent.max_linear, intent.max_vertical, intent.max_angular),
        (3.0, 1.5, 1.5)
    );
    assert_eq!(scope.actions.len(), 2);
    assert_eq!(vehicle.link_loss_actions, vec![LinkLossPolicy::Neutralize]);
    assert!(h.adapter.video_sources().is_empty());
}

#[test]
fn an_arm_betaflight_refuses_names_its_reasons_and_does_not_move_the_switch() {
    let blockers =
        ArmingBlocker::RxLoss.bit() | ArmingBlocker::Angle.bit() | ArmingBlocker::ArmSwitch.bit();
    let mut h = harness(Some(observation(false, blockers, true)));
    let outcome = h
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Arm]));
    let result = &outcome.action_results[0];
    assert!(!result.accepted);
    assert_eq!(result.detail, "Betaflight refuses to arm: RXLOSS, ANGLE");
    let requests = h.commands.borrow().requests;
    assert_eq!((requests.arm, requests.refused), (0, 1));
}

#[test]
fn an_arm_with_no_blocker_raises_a_request() {
    let mut h = harness(Some(observation(false, 0, true)));
    let outcome = h
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Arm]));
    assert!(outcome.action_results[0].accepted);
    assert_eq!(h.commands.borrow().requests.arm, 1);
}

#[test]
fn a_stopped_control_loop_refuses_arming_with_that_reason() {
    let mut stopped = observation(false, ArmingBlocker::BootGraceTime.bit(), true);
    stopped.snapshot.cycle_time_us = 0;
    let mut h = harness(Some(stopped));
    let outcome = h
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Arm]));
    assert!(!outcome.action_results[0].accepted);
    assert!(
        outcome.action_results[0]
            .detail
            .contains("control loop is not running")
    );
    assert_eq!(h.commands.borrow().requests.arm, 0);
}

#[test]
fn a_configuration_issue_refuses_arming_with_the_fix() {
    let mut h = harness(Some(observation(false, 0, true)));
    h.reports.send_modify(|report| {
        report.issue = Some(ConfigIssue::ReceiverNotMsp { features: 0x2 });
    });
    let outcome = h
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Arm]));
    assert!(!outcome.action_results[0].accepted);
    assert!(outcome.action_results[0].detail.contains("RX_MSP"));
}

#[test]
fn disarm_is_accepted_even_without_telemetry() {
    let mut h = harness(None);
    let outcome = h.adapter.apply_control(&frame(
        Some([1.0, 0.0, 0.0, 0.0]),
        vec![ControlAction::Disarm],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    assert!(outcome.action_results[0].accepted);
    assert_eq!(h.commands.borrow().requests.disarm, 1);
    assert!(
        h.commands.borrow().demands.velocity_at.is_none(),
        "the demand was not taken"
    );
}

#[test]
fn a_demand_without_telemetry_is_refused() {
    let mut h = harness(None);
    let outcome = h.adapter.apply_control(&frame(Some([1.0; 4]), vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
}

#[test]
fn a_demand_is_clamped_to_the_envelope_and_handed_to_the_link() {
    let mut h = harness(Some(observation(true, 0, true)));
    let outcome = h
        .adapter
        .apply_control(&frame(Some([5.0, -1.0, -1.0, f32::NAN]), vec![]));
    assert_eq!(outcome.disposition, Disposition::Constrained);
    let command = *h.commands.borrow();
    assert_eq!(command.demands.velocity.forward, 3.0);
    assert_eq!(command.demands.velocity.right, -1.0);
    assert_eq!(command.demands.velocity.up, 1.0, "body-FRD -z is up");
    assert_eq!(command.demands.velocity.yaw_rate, 0.0);
    assert!(command.demands.velocity_at.is_some());
}

#[test]
fn only_body_frd_velocity_is_taken() {
    let mut h = harness(Some(observation(true, 0, true)));
    let mut ned = frame(Some([1.0, 0.0, 0.0, 0.0]), vec![]);
    if let Some(ControlIntent::Velocity(velocity)) = ned.intent.as_mut() {
        velocity.frame = ReferenceFrame::LocalNed;
    }
    assert!(matches!(
        h.adapter.apply_control(&ned).disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
}

#[test]
fn link_loss_holds_and_suppresses_until_cleared() {
    let mut h = harness(Some(observation(true, 0, true)));
    h.adapter
        .apply_control(&frame(Some([2.0, 0.0, 0.0, 0.0]), vec![]));
    let scope = ScopeId::new(FLIGHT_SCOPE);
    h.adapter
        .set_link_loss_policy(VehicleId::new(1), &scope, Some(LinkLossPolicy::Neutralize))
        .expect("engages");
    assert_eq!(h.commands.borrow().demands.velocity.forward, 0.0, "a hold");
    assert_eq!(
        h.adapter
            .apply_control(&frame(Some([2.0, 0.0, 0.0, 0.0]), vec![]))
            .disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
    h.adapter
        .set_link_loss_policy(VehicleId::new(1), &scope, None)
        .expect("clears");
    assert_eq!(
        h.adapter
            .apply_control(&frame(Some([2.0, 0.0, 0.0, 0.0]), vec![]))
            .disposition,
        Disposition::Accepted
    );
}

#[test]
fn a_dead_link_cannot_enact_link_loss_but_still_latches() {
    let Harness {
        mut adapter,
        reports: _reports,
        commands,
    } = harness(Some(observation(true, 0, true)));
    drop(commands);
    let scope = ScopeId::new(FLIGHT_SCOPE);
    assert!(
        adapter
            .set_link_loss_policy(VehicleId::new(1), &scope, Some(LinkLossPolicy::Neutralize))
            .is_err()
    );
    assert_eq!(
        adapter
            .apply_control(&frame(Some([0.0; 4]), vec![]))
            .disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
}

#[test]
fn an_armed_fixed_poll_samples_like_px4() {
    let mut h = harness(Some(observation(true, 0, true)));
    let batch = h.adapter.sample_telemetry();
    let sample = &batch.samples[0];
    let avionics = sample.avionics.expect("avionics");
    let kinematics = avionics.kinematics.expect("kinematics with a fix");
    assert_eq!(kinematics.pos_ned_m, [4.0, -2.0, -3.0]);
    assert_eq!(kinematics.vel_ned_mps, [1.0, 2.0, -0.5]);
    assert_eq!(avionics.valid_flags, 0b1111);
    assert!(avionics.estimator_status_stamp.is_some());
    let fix = avionics.geodetic.expect("geodetic fix");
    assert!((fix.position.latitude_deg - 47.397_742).abs() < 1e-9);
    assert_eq!(sample.fc_state.expect("fc state").arm_state, 2);
    let pose = sample.pose.expect("pose");
    assert_eq!((pose.x, pose.y), (4.0, -2.0));
    assert!(sample.battery.is_none(), "the SITL measures no battery");
}

#[test]
fn without_a_fix_only_attitude_is_valid() {
    let mut h = harness(Some(observation(false, 0, false)));
    let batch = h.adapter.sample_telemetry();
    let avionics = batch.samples[0].avionics.expect("avionics");
    assert!(avionics.kinematics.is_none() && avionics.geodetic.is_none());
    assert_eq!(avionics.valid_flags, 0b0011);
    assert_eq!(batch.samples[0].fc_state.expect("fc state").arm_state, 1);
}

#[test]
fn a_stale_poll_is_withheld() {
    let mut stale = observation(true, 0, true);
    stale.snapshot.received_at = Instant::now()
        .checked_sub(Duration::from_secs(4))
        .expect("monotonic clock is past four seconds");
    let mut h = harness(Some(stale));
    assert!(h.adapter.sample_telemetry().samples.is_empty());
    assert_eq!(
        h.adapter
            .apply_control(&frame(Some([0.0; 4]), vec![]))
            .disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
}

fn direct_frame(intent: ControlIntent) -> ScopedControlFrame {
    ScopedControlFrame {
        scope: ScopeId::new(super::DIRECT_SCOPE),
        intent: Some(intent),
        ..frame(None, vec![])
    }
}

#[test]
fn body_rates_on_the_direct_scope_hand_a_direct_demand_to_the_link() {
    let mut h = harness(Some(observation(true, 0, true)));
    let outcome = h
        .adapter
        .apply_control(&direct_frame(ControlIntent::BodyRate(
            pilotage_protocol::BodyRateIntent {
                roll_rate: 1.0,
                pitch_rate: -0.5,
                yaw_rate: 20.0,
                thrust: 0.6,
            },
        )));
    assert_eq!(
        outcome.disposition,
        Disposition::Constrained,
        "yaw clamps at 670 deg/s"
    );
    let demands = h.commands.borrow().demands;
    assert_eq!(demands.family, crate::link::Family::Direct);
    let Some(crate::direct::DirectDemand::Rates {
        roll, yaw, thrust, ..
    }) = demands.direct
    else {
        panic!("rates expected: {demands:?}");
    };
    assert!((roll - 1.0).abs() < 1e-6 && (thrust - 0.6).abs() < 1e-6);
    assert!((yaw - f64::from(670f32.to_radians())).abs() < 1e-4);
}

#[test]
fn an_attitude_on_the_direct_scope_becomes_angles_and_a_heading() {
    let mut h = harness(Some(observation(true, 0, true)));
    let half = 10f32.to_radians() / 2.0;
    let outcome = h
        .adapter
        .apply_control(&direct_frame(ControlIntent::AttitudeThrust(
            pilotage_protocol::AttitudeThrustIntent {
                frame: ReferenceFrame::LocalNed,
                qw: half.cos(),
                qx: half.sin(),
                qy: 0.0,
                qz: 0.0,
                thrust: 0.5,
            },
        )));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    let Some(crate::direct::DirectDemand::Attitude {
        roll,
        pitch,
        heading,
        ..
    }) = h.commands.borrow().demands.direct
    else {
        panic!("attitude expected");
    };
    assert!((roll.to_degrees() - 10.0).abs() < 1e-3);
    assert!(pitch.abs() < 1e-6 && heading.abs() < 1e-6);
}

#[test]
fn each_scope_takes_only_its_own_families() {
    let mut h = harness(Some(observation(true, 0, true)));
    let velocity_on_direct = ScopedControlFrame {
        scope: ScopeId::new(super::DIRECT_SCOPE),
        ..frame(Some([1.0, 0.0, 0.0, 0.0]), vec![])
    };
    assert!(matches!(
        h.adapter.apply_control(&velocity_on_direct).disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
    let rates_on_flight = ScopedControlFrame {
        scope: ScopeId::new(FLIGHT_SCOPE),
        ..direct_frame(ControlIntent::BodyRate(pilotage_protocol::BodyRateIntent {
            roll_rate: 0.0,
            pitch_rate: 0.0,
            yaw_rate: 0.0,
            thrust: 0.5,
        }))
    };
    assert!(matches!(
        h.adapter.apply_control(&rates_on_flight).disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
}

#[test]
fn a_direct_link_loss_falls_back_to_the_assisted_hold() {
    let mut h = harness(Some(observation(true, 0, true)));
    h.adapter
        .apply_control(&direct_frame(ControlIntent::BodyRate(
            pilotage_protocol::BodyRateIntent {
                roll_rate: 1.0,
                pitch_rate: 0.0,
                yaw_rate: 0.0,
                thrust: 0.6,
            },
        )));
    h.adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(super::DIRECT_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("engages");
    let demands = h.commands.borrow().demands;
    assert_eq!(demands.family, crate::link::Family::Assisted);
    assert!(demands.direct_at.is_none() && demands.velocity_at.is_some());
}
