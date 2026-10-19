#![allow(clippy::expect_used, clippy::panic)]

//! Adapter-boundary behavior over a fed link cache and in-memory lanes:
//! EKF-authorized sampling, the arm path, the velocity gate, the airborne
//! disarm, and the per-scope link-loss latch.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::{mpsc, watch};

use pilotage_adapter_api::{
    CommandOutcome, Disposition, LinkLossPolicy, RejectReason, VehicleAdapter,
};
use pilotage_mavlink::codec::{
    FcMessage, FrameSource, MAV_CMD_COMPONENT_ARM_DISARM, MAV_CMD_DO_SET_MODE, MAV_CMD_NAV_TAKEOFF,
    STATUS_TEXT_LEN,
};
use pilotage_mavlink::link::apply_messages_at;
use pilotage_mavlink::{
    AuthorizationSource, BodyVelocityDemand, LinkState, OffboardSetpoint, OutboundCommand,
};
use pilotage_protocol::{
    ControlAction, ControlIntent, ControlPayload, Generation, ReferenceFrame, ScopeId,
    ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
};

use super::{ArduPilotAdapter, FLIGHT_SCOPE};
use crate::uplink::ArduPilotUplink;

/// The body velocity a setpoint lane holds, if it holds one.
pub(super) fn body_velocity(setpoint: Option<OffboardSetpoint>) -> Option<BodyVelocityDemand> {
    match setpoint? {
        OffboardSetpoint::BodyVelocity(demand) => Some(demand),
        OffboardSetpoint::AttitudeThrust(_) => None,
    }
}

mod direct;

const SOURCE: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};
const HEALTHY_EKF: u16 = 1 | 2 | 4 | 8 | 16 | 32;

pub(super) struct Harness {
    pub(super) adapter: ArduPilotAdapter,
    pub(super) state: Arc<Mutex<LinkState>>,
    pub(super) commands: mpsc::Receiver<OutboundCommand>,
    pub(super) velocity: watch::Receiver<Option<OffboardSetpoint>>,
    pub(super) params: mpsc::Receiver<pilotage_mavlink::OutboundParam>,
    pub(super) time_boot_ms: u32,
}

impl Harness {
    pub(super) fn new() -> Self {
        let state = Arc::new(Mutex::new(LinkState {
            authorization_source: AuthorizationSource::ArduPilotEkfStatus,
            standard_status_max_lag_ms: 300,
            maximum_inter_group_skew_ms: 300,
            ..LinkState::default()
        }));
        let (command_tx, commands) = mpsc::channel(32);
        let (velocity_tx, velocity) = watch::channel(None);
        let uplink = ArduPilotUplink::new(command_tx, velocity_tx, 1, 1);
        let (param_tx, params) = mpsc::channel(32);
        Self {
            adapter: ArduPilotAdapter::from_parts(
                VehicleId::new(1),
                state.clone(),
                uplink,
                param_tx,
            ),
            state,
            commands,
            velocity,
            params,
            time_boot_ms: 10_000,
        }
    }

    pub(super) fn feed(&mut self, messages: &[FcMessage]) {
        let tagged: Vec<_> = messages.iter().map(|m| (SOURCE, *m)).collect();
        apply_messages_at(&self.state, &tagged, 0, 0, Instant::now());
    }

    /// One healthy estimate at `down_m`, plus a heartbeat.
    pub(super) fn estimate(&mut self, down_m: f32, armed: bool) {
        self.time_boot_ms += 50;
        let t = self.time_boot_ms;
        self.feed(&[
            FcMessage::Heartbeat {
                armed,
                custom_mode: 4,
                autopilot: 3,
            },
            FcMessage::AttitudeQuaternion {
                time_boot_ms: t,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                rates_rps: [0.0; 3],
            },
            FcMessage::LocalPositionNed {
                time_boot_ms: t,
                pos_ned_m: [0.0, 0.0, down_m],
                vel_ned_mps: [0.0; 3],
            },
            FcMessage::EkfStatusReport { flags: HEALTHY_EKF },
        ]);
        // The report authorizes the NEXT numeric pair.
        self.time_boot_ms += 10;
        let t = self.time_boot_ms;
        self.feed(&[
            FcMessage::AttitudeQuaternion {
                time_boot_ms: t,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                rates_rps: [0.0; 3],
            },
            FcMessage::LocalPositionNed {
                time_boot_ms: t,
                pos_ned_m: [0.0, 0.0, down_m],
                vel_ned_mps: [0.0; 3],
            },
        ]);
        self.adapter.sample_telemetry();
    }

    fn commands(&mut self) -> Vec<u16> {
        let mut sent = Vec::new();
        while let Ok(command) = self.commands.try_recv() {
            sent.push(command.command);
        }
        sent
    }
}

fn frame(intent: Option<VelocityIntent>, actions: Vec<ControlAction>) -> ScopedControlFrame {
    scoped(FLIGHT_SCOPE, intent.map(ControlIntent::Velocity), actions)
}

pub(super) fn scoped(
    scope: &str,
    intent: Option<ControlIntent>,
    actions: Vec<ControlAction>,
) -> ScopedControlFrame {
    ScopedControlFrame {
        action_ids: (1..=actions.len() as u32).collect(),
        session: SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new(scope),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: pilotage_timing::MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent,
        actions,
    }
}

pub(super) fn velocity(vx: f32, vz: f32, yaw_rate: f32) -> Option<VelocityIntent> {
    Some(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy: 0.0,
        vz,
        yaw_rate,
    })
}

#[test]
fn an_ekf_authorized_estimate_samples_a_pose() {
    let mut harness = Harness::new();
    harness.estimate(-1.0, false);
    let batch = harness.adapter.sample_telemetry();
    let sample = batch.samples.first().expect("a sample");
    assert!(sample.pose.is_some(), "the EKF report authorizes the pose");
    assert_eq!(sample.fc_state.expect("fc state").arm_state, 1);
}

#[test]
fn arm_goes_to_the_fc_without_an_estimate() {
    let mut harness = Harness::new();
    let outcome = harness
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Arm]));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    assert!(outcome.action_results.iter().all(|result| result.accepted));
    assert_eq!(
        harness.commands(),
        vec![MAV_CMD_DO_SET_MODE, MAV_CMD_COMPONENT_ARM_DISARM]
    );
}

#[test]
fn a_refused_arm_reaches_telemetry_as_the_fc_verdict() {
    let mut harness = Harness::new();
    harness.estimate(0.0, false);
    harness
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Arm]));
    let pending = harness.adapter.sample_telemetry().samples[0]
        .fc_state
        .and_then(|state| state.last_command)
        .expect("the arm is answered at once");
    assert_eq!(pending.outcome, CommandOutcome::InProgress);
    let mut text = [0_u8; STATUS_TEXT_LEN];
    text[..18].copy_from_slice(b"PreArm: GPS 1: Bad");
    harness.feed(&[
        FcMessage::StatusText { severity: 3, text },
        FcMessage::CommandAck {
            command: MAV_CMD_COMPONENT_ARM_DISARM,
            result: 4,
            target_system: 255,
            target_component: 190,
        },
    ]);
    let batch = harness.adapter.sample_telemetry();
    let ack = batch.samples[0]
        .fc_state
        .expect("fc state")
        .last_command
        .expect("the verdict");
    assert!(ack.arm);
    assert_eq!(ack.outcome, CommandOutcome::Failed, "MAV_RESULT_FAILED");
}

#[test]
fn velocity_without_a_pose_is_refused() {
    let mut harness = Harness::new();
    let outcome = harness
        .adapter
        .apply_control(&frame(velocity(1.0, 0.0, 0.0), vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
}

#[test]
fn climb_takes_off_then_velocity_streams_body_frame_and_disarm_lands() {
    let mut harness = Harness::new();
    harness.estimate(0.0, true);
    let takeoff = harness
        .adapter
        .apply_control(&frame(velocity(0.0, -1.5, 0.0), vec![]));
    assert_eq!(takeoff.disposition, Disposition::Transformed);
    assert_eq!(harness.commands(), vec![MAV_CMD_NAV_TAKEOFF]);

    harness.estimate(-2.4, true);
    let flying = harness
        .adapter
        .apply_control(&frame(velocity(9.0, 0.0, 0.5), vec![]));
    assert_eq!(flying.disposition, Disposition::Transformed, "9 m/s clamps");
    let demand = body_velocity(*harness.velocity.borrow_and_update()).expect("a setpoint");
    assert_eq!(demand.vel_frd_mps, [3.0, 0.0, 0.0]);
    assert_eq!(demand.yaw_rate_rps, 0.5);

    harness
        .adapter
        .apply_control(&frame(None, vec![ControlAction::Disarm]));
    assert_eq!(
        harness.commands(),
        vec![MAV_CMD_DO_SET_MODE],
        "LAND, not disarm"
    );
}

#[test]
fn link_loss_zeroes_the_setpoint_and_rejects_until_cleared() {
    let mut harness = Harness::new();
    harness.estimate(0.0, true);
    harness
        .adapter
        .apply_control(&frame(velocity(0.0, -1.5, 0.0), vec![]));
    harness.estimate(-2.5, true);
    harness
        .adapter
        .apply_control(&frame(velocity(2.0, 0.0, 0.0), vec![]));
    harness.velocity.borrow_and_update();

    let scope = ScopeId::new(FLIGHT_SCOPE);
    harness
        .adapter
        .set_link_loss_policy(VehicleId::new(1), &scope, Some(LinkLossPolicy::Neutralize))
        .expect("engaged");
    let demand = body_velocity(*harness.velocity.borrow_and_update()).expect("a setpoint");
    assert_eq!(demand.vel_frd_mps, [0.0; 3]);
    let suppressed = harness
        .adapter
        .apply_control(&frame(velocity(2.0, 0.0, 0.0), vec![]));
    assert_eq!(
        suppressed.disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
    assert!(!harness.velocity.has_changed().expect("lane open"));

    harness
        .adapter
        .set_link_loss_policy(VehicleId::new(1), &scope, None)
        .expect("cleared");
    let resumed = harness
        .adapter
        .apply_control(&frame(velocity(2.0, 0.0, 0.0), vec![]));
    assert_eq!(resumed.disposition, Disposition::Accepted);
}

#[test]
fn the_vehicle_advertises_itself_as_a_named_multicopter() {
    let harness = Harness::new();
    let capabilities = harness.adapter.capabilities();
    let vehicle = &capabilities.vehicles[0];
    assert_eq!(vehicle.class, pilotage_protocol::VehicleClass::Multicopter);
    assert_eq!(vehicle.display_name, "ArduPilot quad");
}
