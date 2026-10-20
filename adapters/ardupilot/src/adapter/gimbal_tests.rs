//! Payload control remains independent from flight state and flight authority.
use std::time::{Duration, Instant};

use pilotage_adapter_api::{Disposition, LinkLossPolicy, SourceRole, VehicleAdapter};
use pilotage_mavlink::codec::{
    FcMessage, FrameSource, GCS_COMPONENT_ID, GCS_SYSTEM_ID, GimbalManagerInformation,
    GimbalManagerStatus,
};
use pilotage_mavlink::link::apply_messages_at;
use pilotage_mavlink::{GimbalRateDemand, ManagerControl, OutboundCommand};
use pilotage_protocol::{ControlIntent, GimbalRateIntent, ScopeId, VehicleId};
use tokio::sync::{mpsc, watch};

use super::tests::{Harness, scoped};
use super::{FLIGHT_SCOPE, GIMBAL_SCOPE};

struct PayloadHarness {
    flight: Harness,
    commands: mpsc::Receiver<OutboundCommand>,
    rates: watch::Receiver<Option<GimbalRateDemand>>,
}

impl PayloadHarness {
    fn new() -> Self {
        let mut flight = Harness::new();
        let (commands, command_rx) = mpsc::channel(32);
        let (rates, rate_rx) = watch::channel(None);
        flight.adapter.gimbal = Some(
            ManagerControl::new(commands, rates, 1, None)
                .expect("manager policy")
                .with_manager_state(flight.state.clone()),
        );
        Self {
            flight,
            commands: command_rx,
            rates: rate_rx,
        }
    }

    fn feed(&self, component: u8, message: FcMessage, at: Instant) {
        apply_messages_at(
            &self.flight.state,
            &[(
                FrameSource {
                    system_id: 1,
                    component_id: component,
                    frame_sequence: 0,
                },
                message,
            )],
            0,
            0,
            at,
        );
    }

    fn discover(&mut self, component: u8, device: u8, owner: (u8, u8)) {
        self.feed(
            component,
            FcMessage::GimbalManagerInformation(GimbalManagerInformation {
                time_boot_ms: 100,
                cap_flags: 2 | 32 | 128,
                roll_min: f32::NAN,
                roll_max: f32::NAN,
                pitch_min: -1.5,
                pitch_max: 0.5,
                yaw_min: f32::NAN,
                yaw_max: f32::NAN,
                device_id: device,
            }),
            Instant::now(),
        );
        self.feed(
            component,
            FcMessage::GimbalManagerStatus(GimbalManagerStatus {
                time_boot_ms: 100,
                flags: 0,
                device_id: device,
                primary_system: owner.0,
                primary_component: owner.1,
            }),
            Instant::now(),
        );
        self.flight.adapter.sample_telemetry();
    }

    fn attitude(&self, time: u32, at: Instant) {
        self.feed(
            154,
            FcMessage::GimbalDeviceAttitudeStatus {
                target_system: GCS_SYSTEM_ID,
                target_component: GCS_COMPONENT_ID,
                device_id: 0,
                time_boot_ms: time,
                quat_wxyz: [1.0, 0.0, 0.0, 0.0],
                rates_rps: [f32::NAN, 0.2, f32::NAN],
                flags: 12,
                failure_flags: 2,
            },
            at,
        );
    }

    fn rate(&mut self, pitch: f32, yaw: f32) -> Disposition {
        self.flight
            .adapter
            .apply_control(&scoped(
                GIMBAL_SCOPE,
                Some(ControlIntent::GimbalRate(GimbalRateIntent {
                    pitch_rate: pitch,
                    yaw_rate: yaw,
                })),
                vec![],
            ))
            .disposition
    }
}

#[test]
fn gimbal_is_opt_in_and_unknown_support_is_not_advertised_as_axes() {
    let ordinary = Harness::new();
    assert!(
        ordinary.adapter.capabilities().vehicles[0]
            .scopes
            .iter()
            .all(|scope| scope.scope.as_str() != GIMBAL_SCOPE)
    );
    let harness = PayloadHarness::new();
    let capabilities = harness.flight.adapter.capabilities();
    let scope = capabilities.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == GIMBAL_SCOPE)
        .expect("payload scope");
    assert!(scope.axes.is_empty());
    assert!(scope.intents.is_empty());
    assert!(scope.actions.is_empty());
    assert!(scope.authority_group.is_none());
    assert!(scope.payload.is_some());
}

#[test]
fn pitch_only_manager_controls_without_flight_pose_and_never_maps_yaw_to_aircraft() {
    let mut harness = PayloadHarness::new();
    harness.discover(42, 154, (GCS_SYSTEM_ID, GCS_COMPONENT_ID));
    let capabilities = harness.flight.adapter.capabilities();
    let scope = capabilities.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == GIMBAL_SCOPE)
        .expect("payload scope");
    assert_eq!(
        scope.axes,
        vec![pilotage_protocol::LogicalAxisId::new(super::PITCH_AXIS)]
    );
    assert!(matches!(harness.rate(0.0, 0.2), Disposition::Rejected(_)));
    assert!(harness.rates.borrow().is_none());
    assert_eq!(harness.rate(-0.4, 0.0), Disposition::Accepted);
    let demand = harness.rates.borrow().clone().expect("rate queued");
    assert_eq!(
        (
            demand.target_system,
            demand.target_component,
            demand.gimbal_device_id
        ),
        (1, 42, 154)
    );
    assert_eq!(demand.pitch_rps, -0.4);
    assert!(demand.yaw_rps.is_nan());
    assert!(harness.flight.velocity.borrow().is_none());
    assert!(harness.flight.commands.try_recv().is_err());
}

#[test]
fn payload_attitude_survives_missing_flight_estimate_and_duplicates_keep_their_age() {
    let mut harness = PayloadHarness::new();
    harness.discover(42, 154, (GCS_SYSTEM_ID, GCS_COMPONENT_ID));
    harness.attitude(200, Instant::now());
    let first = harness.flight.adapter.sample_telemetry();
    let sample = &first.samples[0];
    assert!(sample.avionics.is_none());
    let attitude = sample.gimbal.expect("payload attitude");
    assert_eq!(attitude.stamp.role, SourceRole::PayloadDevice);
    assert_eq!(attitude.stamp.source_id, 154 << 8);
    assert_eq!(attitude.stamp.acquired_at_ns, 200_000_000);
    assert_eq!(attitude.failure_flags, 2);
    let payload = sample
        .payload
        .as_ref()
        .expect("portable payload observation");
    let pitch = payload.pitch.as_ref().expect("pitch");
    assert_eq!(pitch.angle_rad, Some(0.0));
    assert_eq!(pitch.rate_rad_s, Some(0.2));
    assert_eq!(payload.yaw.as_ref().expect("yaw angle").rate_rad_s, None);
    harness.attitude(200, Instant::now());
    let repeated = harness.flight.adapter.sample_telemetry();
    assert_eq!(repeated.samples[0].gimbal.unwrap().stamp, attitude.stamp);
    assert_eq!(
        repeated.samples[0]
            .payload
            .as_ref()
            .unwrap()
            .pitch
            .as_ref()
            .unwrap()
            .received_at,
        pitch.received_at
    );
}

#[test]
fn stale_payload_feedback_is_withheld_without_a_fake_flight_pose() {
    let mut harness = PayloadHarness::new();
    harness.discover(42, 154, (GCS_SYSTEM_ID, GCS_COMPONENT_ID));
    harness.attitude(200, Instant::now() - Duration::from_secs(10));
    let batch = harness.flight.adapter.sample_telemetry();
    assert!(
        batch
            .samples
            .iter()
            .all(|sample| sample.gimbal.is_none() && sample.avionics.is_none())
    );
    assert!(
        batch
            .samples
            .iter()
            .all(|sample| sample.payload.as_ref().is_none_or(|p| p.pitch.is_none()))
    );
}

#[test]
fn an_unclaimed_or_foreign_manager_cannot_move_until_ownership_is_reported() {
    let mut harness = PayloadHarness::new();
    harness.discover(42, 154, (0, 0));
    assert!(matches!(harness.rate(0.2, 0.0), Disposition::Rejected(_)));
    assert!(harness.rates.borrow().is_none());
    let mut claim = None;
    while let Ok(command) = harness.commands.try_recv() {
        if command.command == 1001 {
            claim = Some(command);
        }
    }
    let claim = claim.expect("claim queued");
    assert_eq!(claim.target_component, 42);
    assert_eq!(claim.params[6], 154.0);
    assert!(harness.flight.commands.try_recv().is_err());
}

#[test]
fn payload_revocation_fences_queued_motion_without_touching_flight() {
    let mut harness = PayloadHarness::new();
    harness.discover(42, 154, (GCS_SYSTEM_ID, GCS_COMPONENT_ID));
    assert_eq!(harness.rate(0.2, 0.0), Disposition::Accepted);
    let queued = harness.rates.borrow().clone().unwrap();
    harness
        .flight
        .adapter
        .invalidate_pending_commands(VehicleId::new(2), &ScopeId::new(GIMBAL_SCOPE));
    assert!(queued.fence.as_ref().unwrap().is_valid());
    harness
        .flight
        .adapter
        .invalidate_pending_commands(VehicleId::new(1), &ScopeId::new(GIMBAL_SCOPE));
    assert!(!queued.fence.unwrap().is_valid());
    assert!(harness.rates.borrow().is_none());
    assert!(harness.flight.velocity.borrow().is_none());
}

#[test]
fn flight_link_loss_preserves_pending_gimbal_and_payload_latch_is_local() {
    let mut harness = PayloadHarness::new();
    harness.discover(42, 154, (GCS_SYSTEM_ID, GCS_COMPONENT_ID));
    assert_eq!(harness.rate(0.2, 0.0), Disposition::Accepted);
    let sent = harness.rates.borrow().clone().unwrap();
    harness
        .flight
        .adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(FLIGHT_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .unwrap();
    assert_eq!(harness.rates.borrow().as_ref().unwrap().pitch_rps, 0.2);
    let flight_stop = *harness.flight.velocity.borrow();
    assert!(flight_stop.is_none());
    harness
        .flight
        .adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(GIMBAL_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .unwrap();
    assert!(harness.rates.borrow().is_none());
    assert!(!sent.fence.unwrap().is_valid());
    assert_eq!(*harness.flight.velocity.borrow(), flight_stop);
    assert!(matches!(harness.rate(0.2, 0.0), Disposition::Rejected(_)));
}

#[test]
fn discovering_a_free_manager_does_not_claim_it() {
    let mut harness = PayloadHarness::new();
    harness.discover(42, 154, (0, 0));
    let mut requests = 0_u32;
    while let Ok(command) = harness.commands.try_recv() {
        assert_eq!(command.command, 512);
        requests = requests.wrapping_add(1);
    }
    assert!(requests > 0);
    assert!(harness.rates.borrow().is_none());
    assert!(harness.flight.velocity.borrow().is_none());
}
