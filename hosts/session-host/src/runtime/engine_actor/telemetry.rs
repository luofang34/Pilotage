//! Lossless adapter-to-wire telemetry mapping.

use pilotage_adapter_api::{AvionicsSample, MeasurementClock, MeasurementStamp, TelemetrySample};
use pilotage_protocol::wire;
use pilotage_timing::MonoTimestamp;

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
        pose: Some(wire::Pose2d {
            x_m: sample.pose.x as f32,
            y_m: sample.pose.y as f32,
            heading_rad: sample.pose.heading as f32,
        }),
        velocity: Some(wire::Velocity2d {
            linear_x_mps: sample.speed as f32,
            linear_y_mps: 0.0,
            angular_rad_s: 0.0,
        }),
        avionics: sample.avionics.map(avionics_to_wire),
    }
}

fn measurement_stamp_to_wire(stamp: MeasurementStamp) -> wire::MeasurementStamp {
    let clock = match stamp.clock {
        MeasurementClock::VehicleBoot => wire::MeasurementClock::VehicleBoot,
        MeasurementClock::Simulation => wire::MeasurementClock::Simulation,
    };
    wire::MeasurementStamp {
        source_id: stamp.source_id,
        source_epoch: stamp.source_epoch,
        sequence: stamp.sequence,
        acquired_at_ns: stamp.acquired_at_ns,
        clock: clock as i32,
    }
}

fn avionics_to_wire(sample: AvionicsSample) -> wire::AvionicsState {
    wire::AvionicsState {
        quat_w: sample.quat_wxyz[0],
        quat_x: sample.quat_wxyz[1],
        quat_y: sample.quat_wxyz[2],
        quat_z: sample.quat_wxyz[3],
        rate_p_rad_s: sample.rates_rps[0],
        rate_q_rad_s: sample.rates_rps[1],
        rate_r_rad_s: sample.rates_rps[2],
        pos_n_m: sample.pos_ned_m[0],
        pos_e_m: sample.pos_ned_m[1],
        pos_d_m: sample.pos_ned_m[2],
        vel_n_mps: sample.vel_ned_mps[0],
        vel_e_mps: sample.vel_ned_mps[1],
        vel_d_mps: sample.vel_ned_mps[2],
        valid_flags: sample.valid_flags,
        quality: sample.quality,
        arm_state: sample.arm_state,
        attitude_stamp: sample.attitude_stamp.map(measurement_stamp_to_wire),
        kinematics_stamp: sample.kinematics_stamp.map(measurement_stamp_to_wire),
    }
}

#[cfg(test)]
mod tests;
