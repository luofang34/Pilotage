use pilotage_adapter_api::{
    AvionicsAttitudeSample, Disposition, LinkLossPolicy, RejectReason, SourceRole, StepBudget,
    TelemetrySample, VehicleAdapter,
};
use pilotage_protocol::{
    ActionKind, ControlAction, ControlIntent, ControlPayload, Generation, GimbalRateIntent,
    GimbalYawFrame, HeadingReference, IntentFamily, LandedState, ReferenceFrame, ScopeId,
    ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;

use super::{GIMBAL_SCOPE, MOTION_SCOPE, SimMulticopterAdapter};
use crate::config::SimConfig;
use crate::geodesy::LaunchPoint;
use crate::lifecycle::FlightPhase;

const VEHICLE: VehicleId = VehicleId::new(1);

fn adapter() -> SimMulticopterAdapter {
    let mut config = SimConfig::new(LaunchPoint {
        lat_deg: 40.3317,
        lon_deg: -74.6745,
        alt_m: 40.0,
    });
    config.name = "test".to_owned();
    SimMulticopterAdapter::new(VEHICLE, config)
}

fn frame(
    scope: &str,
    intent: Option<ControlIntent>,
    actions: Vec<ControlAction>,
) -> ScopedControlFrame {
    ScopedControlFrame {
        timing: None,
        admitted_timing: None,
        action_ids: vec![],
        session: SessionId::new(1),
        vehicle: VEHICLE,
        scope: ScopeId::new(scope),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent,
        actions,
    }
}

fn velocity(vx: f32, vz: f32, yaw_rate: f32) -> Option<ControlIntent> {
    Some(ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy: 0.0,
        vz,
        yaw_rate,
    }))
}

fn act(adapter: &mut SimMulticopterAdapter, action: ControlAction) -> bool {
    let outcome = adapter.apply_control(&frame(MOTION_SCOPE, None, vec![action]));
    outcome.action_results[0].accepted
}

/// Streams `intent` every 50 ms for `seconds`, as the mission does.
fn fly(adapter: &mut SimMulticopterAdapter, intent: Option<ControlIntent>, seconds: u32) {
    for _ in 0..seconds * 20 {
        let outcome = adapter.apply_control(&frame(MOTION_SCOPE, intent, vec![]));
        assert!(
            !matches!(outcome.disposition, Disposition::Rejected(_)),
            "{:?}",
            outcome.disposition
        );
        adapter.step(StepBudget { ticks: 5 });
    }
}

fn latest_sample(adapter: &mut SimMulticopterAdapter) -> TelemetrySample {
    // A multiple of 100 ticks is due for every publication group.
    adapter.step(StepBudget { ticks: 1 });
    while !adapter.tick.is_multiple_of(100) {
        adapter.step(StepBudget { ticks: 1 });
    }
    adapter
        .sample_telemetry()
        .samples
        .into_iter()
        .next()
        .expect("a sample is due")
}

#[test]
fn the_capabilities_mirror_the_dji_shape_and_state_simulation() {
    let adapter = adapter();
    let capabilities = adapter.capabilities();
    assert!(!capabilities.execution.physically_embodied);
    assert!(capabilities.execution.stepped);
    let vehicle = &capabilities.vehicles[0];
    assert!(vehicle.display_name.starts_with("SIM"));
    assert_eq!(vehicle.heading_reference, Some(HeadingReference::True));
    assert_eq!(vehicle.link_loss_actions, vec![LinkLossPolicy::Neutralize]);
    let motion = vehicle
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == MOTION_SCOPE)
        .expect("a flight scope");
    let intent = &motion.intents[0];
    assert_eq!(intent.family, IntentFamily::Velocity);
    assert_eq!(intent.frames, vec![ReferenceFrame::BodyFrd]);
    assert_eq!((intent.max_linear, intent.max_vertical), (15.0, 4.0));
    assert!((intent.max_angular - 100.0_f32.to_radians()).abs() < 1e-6);
    let actions: Vec<ActionKind> = motion.actions.iter().map(|a| a.action).collect();
    for kind in [
        ActionKind::Takeoff,
        ActionKind::Land,
        ActionKind::Arm,
        ActionKind::Disarm,
    ] {
        assert!(actions.contains(&kind), "{kind:?}");
    }
    let gimbal = vehicle
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == GIMBAL_SCOPE)
        .expect("a gimbal scope");
    assert_eq!(gimbal.intents[0].family, IntentFamily::GimbalRate);
    assert_eq!(gimbal.actions[0].action, ActionKind::GimbalRecenter);
    assert!(
        adapter.video_sources().is_empty(),
        "no camera before subscribe"
    );
}

#[test]
fn velocity_is_refused_while_disarmed() {
    let mut adapter = adapter();
    let outcome = adapter.apply_control(&frame(MOTION_SCOPE, velocity(0.0, -2.0, 0.0), vec![]));
    assert!(matches!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
    adapter.step(StepBudget { ticks: 100 });
    assert_eq!(adapter.kinematics().height_m(), 0.0);
}

#[test]
fn arm_then_a_climb_command_flies_and_the_estimate_follows() {
    let mut adapter = adapter();
    assert!(act(&mut adapter, ControlAction::Arm));
    fly(&mut adapter, velocity(0.0, -3.0, 0.0), 10);
    assert_eq!(adapter.phase(), FlightPhase::Airborne);
    let height = adapter.kinematics().height_m();
    assert!(height > 25.0 && height < 30.0, "{height}");
    let sample = latest_sample(&mut adapter);
    let avionics = sample.avionics.expect("an estimate");
    let fix = avionics.geodetic.expect("a fix");
    let kinematics = avionics.kinematics.expect("kinematics");
    let Some(AvionicsAttitudeSample::Canonical {
        stamp, quat_wxyz, ..
    }) = avionics.attitude
    else {
        panic!("a canonical attitude");
    };
    assert_eq!(stamp, fix.stamp);
    assert_eq!(stamp, kinematics.stamp);
    assert_eq!(Some(stamp), avionics.estimator_status_stamp);
    assert_eq!(stamp.role, SourceRole::OperationalEstimate);
    assert_eq!(avionics.valid_flags & 0b1101, 0b1101);
    let norm: f32 = quat_wxyz.iter().map(|v| v * v).sum();
    assert!((norm - 1.0).abs() < 1e-4);
    let vertical = fix.position.vertical;
    assert!((vertical.height_m - adapter.kinematics().height_m()).abs() < 1e-6);
    assert_ne!(vertical.origin.0, 0);
    let truth = sample.sim_truth.expect("a truth group");
    assert_eq!(truth.stamp.role, SourceRole::SimulationTruth);
    let fc = sample.fc_state.expect("FC state");
    assert_eq!(fc.arm_state, 2);
    assert_eq!(fc.landed_state.map(|s| s.state), Some(LandedState::InAir));
}

#[test]
fn takeoff_hovers_and_land_disarms() {
    let mut adapter = adapter();
    assert!(act(&mut adapter, ControlAction::Takeoff));
    adapter.step(StepBudget { ticks: 1_000 });
    assert_eq!(adapter.phase(), FlightPhase::Airborne);
    assert!((adapter.kinematics().height_m() - 1.2).abs() < 0.15);
    assert!(
        !act(&mut adapter, ControlAction::Disarm),
        "no disarm in flight"
    );
    assert!(act(&mut adapter, ControlAction::Land));
    let outcome = adapter.apply_control(&frame(MOTION_SCOPE, velocity(1.0, 0.0, 0.0), vec![]));
    assert!(matches!(outcome.disposition, Disposition::Rejected(_)));
    adapter.step(StepBudget { ticks: 1_000 });
    assert_eq!(adapter.phase(), FlightPhase::Disarmed);
    assert_eq!(adapter.kinematics().height_m(), 0.0);
    assert!(
        !act(&mut adapter, ControlAction::Land),
        "no land on the ground"
    );
}

#[test]
fn a_stale_command_stops_into_a_hover() {
    let mut adapter = adapter();
    assert!(act(&mut adapter, ControlAction::Arm));
    fly(&mut adapter, velocity(8.0, -2.0, 0.0), 5);
    assert!(adapter.kinematics().ground_speed_mps() > 5.0);
    adapter.step(StepBudget { ticks: 600 });
    assert!(adapter.kinematics().ground_speed_mps() < 0.05);
    assert!(adapter.kinematics().vel_ned_mps[2].abs() < 0.05);
}

#[test]
fn an_engaged_link_loss_policy_fences_only_its_scope() {
    let mut adapter = adapter();
    assert!(act(&mut adapter, ControlAction::Arm));
    adapter
        .set_link_loss_policy(
            VEHICLE,
            &ScopeId::new(MOTION_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("the policy engages");
    let outcome = adapter.apply_control(&frame(MOTION_SCOPE, velocity(1.0, -1.0, 0.0), vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
    let gimbal = adapter.apply_control(&frame(
        GIMBAL_SCOPE,
        Some(ControlIntent::GimbalRate(GimbalRateIntent {
            pitch_rate: 0.2,
            yaw_rate: 0.0,
        })),
        vec![],
    ));
    assert_eq!(gimbal.disposition, Disposition::Accepted);
    adapter
        .set_link_loss_policy(VEHICLE, &ScopeId::new(MOTION_SCOPE), None)
        .expect("the policy clears");
    let outcome = adapter.apply_control(&frame(MOTION_SCOPE, velocity(1.0, -1.0, 0.0), vec![]));
    assert_eq!(outcome.disposition, Disposition::Accepted);
}

#[test]
fn the_gimbal_pitches_under_rate_control_and_recenters() {
    let mut adapter = adapter();
    let start = adapter.gimbal_pitch_rad();
    assert!(
        (start + core::f64::consts::FRAC_PI_2).abs() < 1e-9,
        "nadir by default"
    );
    for _ in 0..20 {
        let outcome = adapter.apply_control(&frame(
            GIMBAL_SCOPE,
            Some(ControlIntent::GimbalRate(GimbalRateIntent {
                pitch_rate: 0.3,
                yaw_rate: 0.0,
            })),
            vec![],
        ));
        assert_eq!(outcome.disposition, Disposition::Accepted);
        adapter.step(StepBudget { ticks: 5 });
    }
    assert!((adapter.gimbal_pitch_rad() - (start + 0.3)).abs() < 0.02);
    let outcome = adapter.apply_control(&frame(
        GIMBAL_SCOPE,
        None,
        vec![ControlAction::GimbalRecenter],
    ));
    assert!(outcome.action_results[0].accepted);
    adapter.step(StepBudget { ticks: 500 });
    assert_eq!(adapter.gimbal_pitch_rad(), 0.0);
    let sample = latest_sample(&mut adapter);
    let gimbal = sample.gimbal.expect("a gimbal attitude");
    assert_eq!(gimbal.reference.yaw_frame, GimbalYawFrame::Earth);
    assert!(gimbal.reference.locks.roll && gimbal.reference.locks.pitch);
    assert!(!gimbal.reference.locks.yaw);
    assert_eq!(gimbal.stamp.role, SourceRole::PayloadDevice);
}

#[test]
fn flight_drains_the_battery() {
    let mut adapter = adapter();
    let full = latest_sample(&mut adapter)
        .battery
        .and_then(|battery| battery.remaining_fraction)
        .expect("a battery report");
    assert_eq!(full, 1.0);
    assert!(act(&mut adapter, ControlAction::Arm));
    fly(&mut adapter, velocity(10.0, -1.0, 0.0), 60);
    let battery = latest_sample(&mut adapter)
        .battery
        .expect("a battery report");
    let remaining = battery.remaining_fraction.expect("a fraction");
    assert!(remaining < 0.975 && remaining > 0.95, "{remaining}");
    assert!(battery.current_a.expect("a current") > 3.0);
    assert_eq!(battery.stamp.role, SourceRole::FcState);
}

#[test]
fn the_reported_attitude_uses_the_dji_quaternion_convention() {
    let mut config = SimConfig::new(LaunchPoint {
        lat_deg: 40.3317,
        lon_deg: -74.6745,
        alt_m: 40.0,
    });
    config.initial_heading_deg = 90.0;
    let mut adapter = SimMulticopterAdapter::new(VEHICLE, config);
    let sample = latest_sample(&mut adapter);
    let Some(AvionicsAttitudeSample::Canonical { quat_wxyz, .. }) =
        sample.avionics.and_then(|avionics| avionics.attitude)
    else {
        panic!("a canonical attitude");
    };
    let half = 0.5_f32.sqrt();
    for (got, want) in quat_wxyz.iter().zip([half, 0.0, 0.0, half]) {
        assert!((got - want).abs() < 1e-6, "{quat_wxyz:?}");
    }
}

#[test]
fn foreign_vehicles_scopes_and_payload_frames_are_refused() {
    let mut adapter = adapter();
    let mut foreign = frame(MOTION_SCOPE, velocity(0.0, 0.0, 0.0), vec![]);
    foreign.vehicle = VehicleId::new(9);
    assert_eq!(
        adapter.apply_control(&foreign).disposition,
        Disposition::Rejected(RejectReason::UnknownVehicle)
    );
    assert_eq!(
        adapter
            .apply_control(&frame("vehicle.camera", None, vec![]))
            .disposition,
        Disposition::Rejected(RejectReason::UnknownScope)
    );
    let outcome = adapter.apply_control(&frame(GIMBAL_SCOPE, None, vec![ControlAction::Arm]));
    assert!(!outcome.action_results[0].accepted);
}

#[test]
fn a_grounded_vehicle_takes_a_neutral_frame_and_refuses_motion() {
    let mut adapter = adapter();
    let neutral = adapter.apply_control(&frame(MOTION_SCOPE, velocity(0.0, 0.0, 0.0), vec![]));
    assert_eq!(neutral.disposition, Disposition::Accepted);
    let motion = adapter.apply_control(&frame(MOTION_SCOPE, velocity(1.0, 0.0, 0.0), vec![]));
    assert!(
        matches!(motion.disposition, Disposition::Rejected(_)),
        "{:?}",
        motion.disposition
    );
}
