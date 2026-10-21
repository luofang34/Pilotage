use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{Disposition, SourceIncarnation};
use pilotage_protocol::{
    ControlAction, ControlIntent, ControlPayload, Generation, GimbalRateIntent, ScopeId,
    ScopedControlFrame, SequenceNum, SessionId, VehicleId, wire,
};
use pilotage_timing::{MonoTimestamp, SimTick};
use tokio::sync::{mpsc, watch};

use super::*;
use crate::codec::{
    FcMessage, FrameSource, GCS_COMPONENT_ID, GCS_SYSTEM_ID, GimbalManagerInformation,
    GimbalManagerStatus,
};
use crate::{GimbalRateDemand, LinkState, ManagerControl, OutboundCommand};

struct Rig {
    state: Arc<Mutex<LinkState>>,
    control: ManagerControl,
    commands: mpsc::Receiver<OutboundCommand>,
    rates: watch::Receiver<Option<GimbalRateDemand>>,
}

impl Rig {
    fn new() -> Self {
        let state = Arc::new(Mutex::new(LinkState::default()));
        let (commands, receiver) = mpsc::channel(16);
        let (rates, observer) = watch::channel(None);
        let control = ManagerControl::new(commands, rates, 1, Some((1, 2)))
            .unwrap()
            .with_manager_state(state.clone());
        Self {
            state,
            control,
            commands: receiver,
            rates: observer,
        }
    }

    fn receive(&self, component_id: u8, message: FcMessage, at: Instant) {
        crate::link::apply_messages_at(
            &self.state,
            &[(
                FrameSource {
                    system_id: 1,
                    component_id,
                    frame_sequence: 0,
                },
                message,
            )],
            0,
            0,
            at,
        );
    }

    fn discover(&mut self, flags: u32) {
        self.receive(
            1,
            FcMessage::GimbalManagerInformation(information(flags)),
            Instant::now(),
        );
        self.receive(
            1,
            FcMessage::GimbalManagerStatus(GimbalManagerStatus {
                time_boot_ms: 1,
                flags: 0,
                device_id: 2,
                primary_system: GCS_SYSTEM_ID,
                primary_component: GCS_COMPONENT_ID,
            }),
            Instant::now(),
        );
        self.control.refresh_discovery();
    }

    fn feedback(&self, time: u32, rates: [f32; 3], at: Instant) {
        self.orientation(time, [1.0, 0.0, 0.0, 0.0], rates, at);
    }

    fn orientation(&self, time: u32, quaternion: [f32; 4], rates: [f32; 3], at: Instant) {
        self.receive(
            1,
            FcMessage::GimbalDeviceAttitudeStatus {
                time_boot_ms: time,
                quat_wxyz: quaternion,
                rates_rps: rates,
                flags: 12,
                failure_flags: 2,
                target_system: GCS_SYSTEM_ID,
                target_component: GCS_COMPONENT_ID,
                device_id: 2,
            },
            at,
        );
    }
}

fn information(flags: u32) -> GimbalManagerInformation {
    GimbalManagerInformation {
        time_boot_ms: 1,
        cap_flags: flags,
        roll_min: f32::NAN,
        roll_max: f32::NAN,
        pitch_min: -1.5,
        pitch_max: 1.5,
        yaw_min: -3.0,
        yaw_max: 3.0,
        device_id: 2,
    }
}

fn frame() -> ScopedControlFrame {
    ScopedControlFrame {
        timing: None,
        admitted_timing: None,
        session: SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new(SCOPE),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 0,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent: Some(ControlIntent::GimbalRate(GimbalRateIntent {
            pitch_rate: 0.4,
            yaw_rate: 0.0,
        })),
        actions: vec![],
        action_ids: vec![],
    }
}

fn stamp() -> GimbalStamp {
    GimbalStamp::new(SourceIncarnation::new([7; 16]))
}

#[test]
fn discovery_projects_only_usable_axes_and_native_actions() {
    let mut rig = Rig::new();
    let unknown = scope_descriptor(&rig.control, &stamp());
    assert!(unknown.axes.is_empty());
    assert!(unknown.actions.is_empty());
    assert_eq!(unknown.authority_group, None);
    rig.discover(2 | 32 | 128);
    let pitch_only = scope_descriptor(&rig.control, &stamp());
    assert_eq!(
        pitch_only.axes,
        vec![pilotage_protocol::LogicalAxisId::new(PITCH_AXIS)]
    );
    assert_eq!(pitch_only.actions.len(), 1);
    let payload = pitch_only.payload.unwrap();
    assert_eq!(
        payload.connection_role,
        wire::PayloadConnectionRole::MavlinkManager as i32
    );
    assert_eq!(payload.resource_scopes, [SCOPE]);
    assert_eq!(payload.connection_incarnation, [7; 16]);
    let pitch = payload.gimbal.unwrap().pitch.unwrap();
    let position = pitch.position.unwrap();
    assert_eq!(
        position.support,
        wire::PayloadSupportState::Unimplemented as i32
    );
    assert!(position.unavailable_reason.is_empty());
    assert_eq!(position.minimum_rad, Some(-1.5));
    assert_eq!(position.maximum_rad, Some(1.5));
    assert_eq!(
        pitch.rate.unwrap().frames,
        [wire::PayloadReferenceFrame::HorizonVehicleYaw as i32]
    );
}

#[test]
fn invalid_mixed_frames_do_not_claim_or_send_any_payload() {
    let mut rig = Rig::new();
    rig.discover(2 | 32 | 128 | 256 | 512);
    let mut invalid = frame();
    invalid.actions.push(ControlAction::GimbalRecenter);
    invalid.intent = Some(ControlIntent::Velocity(pilotage_protocol::VelocityIntent {
        frame: pilotage_protocol::ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    }));
    let result = apply_control(
        &mut rig.control,
        VehicleId::new(1),
        &invalid,
        SimTick::new(0),
    );
    assert!(matches!(result.disposition, Disposition::Rejected(_)));
    assert_eq!(result.action_results.len(), 1);
    invalid = frame();
    invalid.actions.push(ControlAction::Arm);
    assert!(matches!(
        apply_control(
            &mut rig.control,
            VehicleId::new(1),
            &invalid,
            SimTick::new(0)
        )
        .disposition,
        Disposition::Rejected(_)
    ));
    invalid.actions = vec![ControlAction::GimbalRecenter; 2];
    assert!(matches!(
        apply_control(
            &mut rig.control,
            VehicleId::new(1),
            &invalid,
            SimTick::new(0)
        )
        .disposition,
        Disposition::Rejected(_)
    ));
    assert!(rig.commands.try_recv().is_err());
    assert!(rig.rates.borrow().is_none());
    assert!(rig.control.snapshot_cached().is_none());
}

#[test]
fn unknown_axis_demand_and_nonfinite_rates_are_rejected() {
    let mut rig = Rig::new();
    rig.discover(32 | 128);
    let mut request = frame();
    request.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
        pitch_rate: f32::NAN,
        yaw_rate: 0.0,
    }));
    assert!(matches!(
        apply_control(
            &mut rig.control,
            VehicleId::new(1),
            &request,
            SimTick::new(0)
        )
        .disposition,
        Disposition::Rejected(_)
    ));
    assert!(rig.commands.try_recv().is_err());
    request.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
        pitch_rate: 0.0,
        yaw_rate: 0.4,
    }));
    assert!(matches!(
        apply_control(
            &mut rig.control,
            VehicleId::new(1),
            &request,
            SimTick::new(0)
        )
        .disposition,
        Disposition::Rejected(_)
    ));
    assert!(rig.rates.borrow().is_none());
}

#[test]
fn valid_rate_frame_scales_without_aircraft_authorization() {
    let mut rig = Rig::new();
    rig.discover(32 | 128);
    let result = apply_control(
        &mut rig.control,
        VehicleId::new(1),
        &frame(),
        SimTick::new(0),
    );
    assert_eq!(result.disposition, Disposition::Accepted);
    assert_eq!(rig.rates.borrow().as_ref().unwrap().pitch_rps, 0.4);
    assert!(rig.state.lock().unwrap().estimator_status.is_none());
}

#[test]
fn projections_preserve_one_report_identity_and_do_not_refresh_it() {
    let mut rig = Rig::new();
    rig.discover(32 | 128);
    rig.feedback(2, [f32::NAN, 0.25, 0.0], Instant::now());
    let first = sample(&rig.control, &stamp());
    let repeated = sample(&rig.control, &stamp());
    let attitude = first.attitude.unwrap();
    let payload = first.payload.unwrap();
    let pitch = payload.pitch.unwrap();
    assert_eq!(
        pitch.stamp.as_ref().unwrap().source_id,
        attitude.stamp.source_id
    );
    assert_eq!(
        pitch.stamp.as_ref().unwrap().sequence,
        attitude.stamp.sequence
    );
    assert_eq!(
        pitch.stamp.as_ref().unwrap().acquired_at_ns,
        attitude.stamp.acquired_at_ns
    );
    assert_eq!(pitch.rate_rad_s, Some(0.25));
    assert_eq!(payload.roll.unwrap().rate_rad_s, None);
    assert_eq!(attitude.failure_flags, 2);
    assert_eq!(repeated.attitude.unwrap().stamp, attitude.stamp);
    assert_eq!(repeated.payload.unwrap().pitch.unwrap(), pitch);
}

#[test]
fn stale_observations_are_withheld_without_erasing_command_progress() {
    let mut rig = Rig::new();
    rig.discover(32 | 128);
    rig.feedback(2, [0.0; 3], Instant::now() - Duration::from_secs(6));
    assert!(sample(&rig.control, &stamp()).attitude.is_none());
    assert!(sample(&rig.control, &stamp()).payload.is_none());
    assert_eq!(
        apply_control(
            &mut rig.control,
            VehicleId::new(1),
            &frame(),
            SimTick::new(0)
        )
        .disposition,
        Disposition::Accepted
    );
    let report = sample(&rig.control, &stamp());
    assert!(report.attitude.is_none());
    let payload = report.payload.unwrap();
    assert!(payload.pitch.is_none());
    assert!(payload.command.is_some());
}

#[test]
fn claim_denial_is_component_specific_and_in_progress_is_not_a_denial() {
    let mut rig = Rig::new();
    rig.discover(32 | 128);
    rig.receive(
        191,
        FcMessage::GimbalManagerInformation(information(32 | 128)),
        Instant::now(),
    );
    let ack = |result| FcMessage::CommandAck {
        command: 1001,
        result,
        target_system: GCS_SYSTEM_ID,
        target_component: GCS_COMPONENT_ID,
    };
    rig.receive(191, ack(4), Instant::now());
    assert!(
        rig.control
            .claim_denial_cached(Duration::from_secs(5))
            .is_none()
    );
    rig.receive(1, ack(4), Instant::now());
    assert_eq!(
        rig.control.claim_denial_cached(Duration::from_secs(5)),
        Some(4)
    );
    rig.receive(1, ack(5), Instant::now());
    assert!(
        rig.control
            .claim_denial_cached(Duration::from_secs(5))
            .is_none()
    );
}

#[test]
fn invalid_quaternion_withholds_orientation_but_keeps_independent_rates() {
    let mut rig = Rig::new();
    rig.discover(32 | 128);
    for (index, quaternion) in [
        [0.0; 4],
        [2.0, 0.0, 0.0, 0.0],
        [f32::NAN, 0.0, 0.0, 0.0],
        [f32::INFINITY, 0.0, 0.0, 0.0],
    ]
    .into_iter()
    .enumerate()
    {
        rig.orientation(
            2 + index as u32,
            quaternion,
            [0.3, f32::NAN, f32::NAN],
            Instant::now(),
        );
        let observation = sample(&rig.control, &stamp());
        assert!(observation.attitude.is_none());
        let payload = observation.payload.unwrap();
        assert!(payload.pitch.is_none());
        assert!(payload.yaw.is_none());
        let roll = payload.roll.unwrap();
        assert_eq!(roll.angle_rad, None);
        assert_eq!(roll.rate_rad_s, Some(0.3));
    }
}

#[test]
fn invalid_quaternion_without_rates_does_not_fabricate_a_level_attitude() {
    let mut rig = Rig::new();
    rig.discover(32 | 128);
    rig.orientation(2, [0.0; 4], [f32::NAN; 3], Instant::now());
    let observation = sample(&rig.control, &stamp());
    assert!(observation.attitude.is_none());
    assert!(observation.payload.is_none());
    rig.orientation(3, [0.99, 0.0, 0.0, 0.0], [0.0; 3], Instant::now());
    let normalized = sample(&rig.control, &stamp());
    assert_eq!(normalized.attitude.unwrap().quat_wxyz, [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(
        normalized.payload.unwrap().pitch.unwrap().angle_rad,
        Some(0.0)
    );
}

#[test]
fn an_axis_with_no_entrypoint_frame_preserves_its_unimplemented_contract() {
    let mut rig = Rig::new();
    rig.discover(32 | 64);
    let descriptor = scope_descriptor(&rig.control, &stamp());
    assert!(descriptor.intents.is_empty());
    assert!(descriptor.axes.is_empty());
    let rate = descriptor
        .payload
        .unwrap()
        .gimbal
        .unwrap()
        .pitch
        .unwrap()
        .rate
        .unwrap();
    assert_eq!(
        rate.support,
        wire::PayloadSupportState::Unimplemented as i32
    );
    assert!(rate.unavailable_reason.is_empty());
    assert_eq!(rate.frames, [wire::PayloadReferenceFrame::BodyFrd as i32]);
}
