//! Shared fixtures: a populated MAVLink estimate cache and a canonical
//! flight control frame.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole,
};
use pilotage_protocol::VehicleId;

use pilotage_mavlink::link::estimator::{EstimatorAuthorization, EstimatorStatusUpdate};
use pilotage_mavlink::link::{AttitudeUpdate, KinematicsUpdate, LinkState};

pub(super) fn state_with(att_age: Duration, kin_age: Duration) -> Arc<Mutex<LinkState>> {
    state_with_acquisition_skew(att_age, kin_age, 0)
}

pub(super) fn state_with_acquisition_skew(
    att_age: Duration,
    kin_age: Duration,
    acquisition_skew_ns: u64,
) -> Arc<Mutex<LinkState>> {
    let now = Instant::now();
    let attitude_stamp = MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 1,
        source_incarnation: SourceIncarnation::new([1; 16]),
        source_epoch: 1,
        sequence: 10,
        acquired_at_ns: 5_000_000_000,
        clock: MeasurementClock::VehicleBoot,
    };
    let kinematics_stamp = MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        integrity: SourceIntegrity::ChecksummedOnly,
        sequence: 5,
        acquired_at_ns: attitude_stamp
            .acquired_at_ns
            .saturating_sub(acquisition_skew_ns),
        ..attitude_stamp
    };
    let skew_ms = u32::try_from(acquisition_skew_ns / 1_000_000).unwrap_or(u32::MAX);
    let estimator_status = EstimatorStatusUpdate {
        time_usec: 5_000_000,
        time_boot_ms: 5_000,
        authorization: EstimatorAuthorization {
            valid_flags: 0b1111,
            quality: 0,
        },
        stamp: MeasurementStamp {
            role: SourceRole::OperationalEstimate,
            integrity: SourceIntegrity::ChecksummedOnly,
            sequence: 7,
            ..attitude_stamp
        },
    };
    let state = LinkState {
        attitude: Some(AttitudeUpdate {
            // 90° yaw: heading east.
            quat_wxyz: [
                core::f32::consts::FRAC_1_SQRT_2,
                0.0,
                0.0,
                core::f32::consts::FRAC_1_SQRT_2,
            ],
            rates_rps: [0.0, 0.0, 0.1],
            time_boot_ms: 5000,
            stamp: attitude_stamp,
            valid_flags: 0b1111,
            quality: 0,
            received_at: now.checked_sub(att_age).unwrap_or(now),
        }),
        kinematics: Some(KinematicsUpdate {
            pos_ned_m: [10.0, 20.0, -30.0],
            vel_ned_mps: [3.0, 4.0, -1.0],
            time_boot_ms: 5000_u32.saturating_sub(skew_ms),
            stamp: kinematics_stamp,
            valid_flags: 0b1111,
            quality: 0,
            received_at: now.checked_sub(kin_age).unwrap_or(now),
        }),
        estimator_status: Some(estimator_status),
        maximum_inter_group_skew_ms: 300,
        ..LinkState::default()
    };
    Arc::new(Mutex::new(state))
}

pub(super) fn flight_frame(
    axes: Vec<(pilotage_protocol::LogicalAxisId, f32)>,
    edges: Vec<(
        pilotage_protocol::LogicalButtonId,
        pilotage_protocol::ButtonEdge,
    )>,
) -> pilotage_protocol::ScopedControlFrame {
    pilotage_protocol::ScopedControlFrame {
        session: pilotage_protocol::SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: pilotage_protocol::ScopeId::new(crate::adapter::FLIGHT_SCOPE),
        generation: pilotage_protocol::Generation::new(1),
        sequence: pilotage_protocol::SequenceNum::new(1),
        sampled_at: pilotage_timing::MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        payload: pilotage_protocol::ControlPayload { axes, edges },
    }
}
