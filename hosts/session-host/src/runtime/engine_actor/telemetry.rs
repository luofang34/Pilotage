//! Lossless adapter-to-wire telemetry mapping.

use pilotage_adapter_api::{
    AvionicsSample, FcStateSample, GimbalAttitudeSample, MeasurementClock, MeasurementStamp,
    SimTruthSample, SourceIntegrity, SourceRole, TelemetrySample,
};
use pilotage_protocol::wire;
use pilotage_timing::MonoTimestamp;

const ESTIMATOR_VALID_FLAGS_MASK: u32 = 0x0f;
const ESTIMATOR_QUALITY_UNUSABLE: u32 = 2;

pub(super) fn sample_to_wire(
    sample: TelemetrySample,
    published_at: MonoTimestamp,
) -> wire::TelemetrySample {
    wire::TelemetrySample {
        vehicle: Some(wire::VehicleId {
            value: sample.vehicle.as_u64(),
        }),
        tick: Some(wire::SimTick {
            value: sample.tick.as_u64(),
        }),
        observed_at: Some(wire::MonoTimestamp {
            nanos: published_at.as_nanos(),
        }),
        pose: sample.pose.map(|pose| wire::Pose2d {
            x_m: pose.x as f32,
            y_m: pose.y as f32,
            heading_rad: pose.heading as f32,
        }),
        velocity: sample.speed.map(|speed| wire::Velocity2d {
            linear_x_mps: speed as f32,
            linear_y_mps: 0.0,
            angular_rad_s: 0.0,
        }),
        avionics: sample.avionics.map(avionics_to_wire),
        sim_truth: sample
            .sim_truth
            .map(|truth| Box::new(sim_truth_to_wire(truth))),
        fc_state: sample
            .fc_state
            .map(|state| Box::new(fc_state_to_wire(state))),
        gimbal: sample.gimbal.map(|gimbal| Box::new(gimbal_to_wire(gimbal))),
    }
}

fn gimbal_to_wire(sample: GimbalAttitudeSample) -> wire::GimbalAttitude {
    wire::GimbalAttitude {
        quat_w: sample.quat_wxyz[0],
        quat_x: sample.quat_wxyz[1],
        quat_y: sample.quat_wxyz[2],
        quat_z: sample.quat_wxyz[3],
        rate_x_rad_s: sample.rates_rps[0],
        rate_y_rad_s: sample.rates_rps[1],
        rate_z_rad_s: sample.rates_rps[2],
        stamp: Some(measurement_stamp_to_wire(sample.stamp)),
        flags: sample.flags,
        failure_flags: sample.failure_flags,
    }
}

fn measurement_stamp_to_wire(stamp: MeasurementStamp) -> wire::MeasurementStamp {
    let clock = match stamp.clock {
        MeasurementClock::VehicleBoot => wire::MeasurementClock::VehicleBoot,
        MeasurementClock::Simulation => wire::MeasurementClock::Simulation,
        MeasurementClock::HostMonotonic => wire::MeasurementClock::HostMonotonic,
    };
    let role = match stamp.role {
        SourceRole::OperationalEstimate => wire::SourceRole::OperationalEstimate,
        SourceRole::SimulationTruth => wire::SourceRole::SimulationTruth,
        SourceRole::FcState => wire::SourceRole::FcState,
        SourceRole::VideoCapture => wire::SourceRole::VideoCapture,
        SourceRole::PayloadDevice => wire::SourceRole::PayloadDevice,
    };
    let integrity = match stamp.integrity {
        SourceIntegrity::Authenticated => wire::SourceIntegrity::Authenticated,
        SourceIntegrity::ChecksummedOnly => wire::SourceIntegrity::ChecksummedOnly,
        SourceIntegrity::Unprotected => wire::SourceIntegrity::Unprotected,
    };
    wire::MeasurementStamp {
        role: role as i32,
        integrity: integrity as i32,
        source_id: stamp.source_id,
        source_epoch: stamp.source_epoch,
        sequence: stamp.sequence,
        acquired_at_ns: stamp.acquired_at_ns,
        clock: clock as i32,
        source_incarnation: stamp.source_incarnation.into_bytes().to_vec(),
    }
}

// The deprecated wire lane `arm_state` is initialized (to 0, unknown) but
// never populated: FC-owned arm state travels as TelemetrySample.fc_state.
#[allow(deprecated)]
fn avionics_to_wire(sample: AvionicsSample) -> wire::AvionicsState {
    let attitude = sample.attitude;
    let kinematics = sample.kinematics;
    let (valid_flags, quality) = if sample.estimator_status_stamp.is_some() {
        (
            sample.valid_flags & ESTIMATOR_VALID_FLAGS_MASK,
            sample.quality.min(ESTIMATOR_QUALITY_UNUSABLE),
        )
    } else {
        (0, ESTIMATOR_QUALITY_UNUSABLE)
    };
    wire::AvionicsState {
        quat_w: attitude.map_or(0.0, |group| group.quat_wxyz[0]),
        quat_x: attitude.map_or(0.0, |group| group.quat_wxyz[1]),
        quat_y: attitude.map_or(0.0, |group| group.quat_wxyz[2]),
        quat_z: attitude.map_or(0.0, |group| group.quat_wxyz[3]),
        rate_p_rad_s: attitude.map_or(0.0, |group| group.rates_rps[0]),
        rate_q_rad_s: attitude.map_or(0.0, |group| group.rates_rps[1]),
        rate_r_rad_s: attitude.map_or(0.0, |group| group.rates_rps[2]),
        pos_n_m: kinematics.map_or(0.0, |group| group.pos_ned_m[0]),
        pos_e_m: kinematics.map_or(0.0, |group| group.pos_ned_m[1]),
        pos_d_m: kinematics.map_or(0.0, |group| group.pos_ned_m[2]),
        vel_n_mps: kinematics.map_or(0.0, |group| group.vel_ned_mps[0]),
        vel_e_mps: kinematics.map_or(0.0, |group| group.vel_ned_mps[1]),
        vel_d_mps: kinematics.map_or(0.0, |group| group.vel_ned_mps[2]),
        valid_flags,
        quality,
        // FC-owned arm state travels as TelemetrySample.fc_state with its
        // own provenance; this legacy lane stays at 0 (unknown) so an
        // unstamped copy is never merged into the estimate.
        arm_state: 0,
        attitude_stamp: attitude.map(|group| measurement_stamp_to_wire(group.stamp)),
        kinematics_stamp: kinematics.map(|group| measurement_stamp_to_wire(group.stamp)),
        estimator_status_stamp: sample.estimator_status_stamp.map(measurement_stamp_to_wire),
    }
}

fn sim_truth_to_wire(sample: SimTruthSample) -> wire::SimTruthState {
    wire::SimTruthState {
        quat_w: sample.quat_wxyz[0],
        quat_x: sample.quat_wxyz[1],
        quat_y: sample.quat_wxyz[2],
        quat_z: sample.quat_wxyz[3],
        pos_n_m: sample.pos_ned_m[0],
        pos_e_m: sample.pos_ned_m[1],
        pos_d_m: sample.pos_ned_m[2],
        vel_n_mps: sample.vel_ned_mps[0],
        vel_e_mps: sample.vel_ned_mps[1],
        vel_d_mps: sample.vel_ned_mps[2],
        valid_flags: sample.valid_flags,
        stamp: Some(measurement_stamp_to_wire(sample.stamp)),
    }
}

fn fc_state_to_wire(sample: FcStateSample) -> wire::FcState {
    wire::FcState {
        arm_state: sample.arm_state,
        // 0 none observed, 1 arm, 2 disarm — the FC's COMMAND_ACK verdict
        // for the most recent commanded arm/disarm (enactment truth).
        last_command_kind: sample
            .last_command
            .map_or(0, |ack| if ack.arm { 1 } else { 2 }),
        last_command_result: sample.last_command.map_or(0, |ack| ack.result),
        stamp: Some(measurement_stamp_to_wire(sample.stamp)),
    }
}

#[cfg(test)]
mod tests;
