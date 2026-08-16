//! Wire-to-feeder conversions.
//!
//! The wire message is prost-generated; the feeder speaks its own typed
//! sample. The conversion is total and judgement-free: absent stamps stay
//! absent, and the feeder's admission rules — not this module — decide
//! what displays.

use indicate_instrument_feeder::RawStamp;
use indicate_instrument_feeder::avionics::{AttitudeGroup, AvionicsSample, KinematicsGroup};
use pilotage_protocol::wire;

/// Converts one wire measurement stamp. `None` in, `None` out: a missing
/// stamp means the group was not supplied, and inventing one would turn
/// absence into a measurement.
#[must_use]
pub fn raw_stamp(stamp: Option<&wire::MeasurementStamp>) -> Option<RawStamp> {
    let stamp = stamp?;
    let mut incarnation = [0_u8; 16];
    if stamp.source_incarnation.len() == 16 {
        incarnation.copy_from_slice(&stamp.source_incarnation);
    }
    Some(RawStamp {
        role: clamp_u8(stamp.role),
        integrity: clamp_u8(stamp.integrity),
        source_id: stamp.source_id,
        incarnation,
        epoch: stamp.source_epoch,
        sequence: stamp.sequence,
        acquired_at_ns: stamp.acquired_at_ns,
        clock: clamp_u8(stamp.clock),
    })
}

/// Converts one wire avionics publication into the feeder's sample.
#[must_use]
pub fn avionics_sample(vehicle_id: u64, avionics: &wire::AvionicsState) -> AvionicsSample {
    AvionicsSample {
        vehicle_id,
        attitude: AttitudeGroup {
            quat: [
                avionics.quat_w,
                avionics.quat_x,
                avionics.quat_y,
                avionics.quat_z,
            ],
            rates: [
                avionics.rate_p_rad_s,
                avionics.rate_q_rad_s,
                avionics.rate_r_rad_s,
            ],
            arm_state: 0,
        },
        kinematics: KinematicsGroup {
            pos_ned: [avionics.pos_n_m, avionics.pos_e_m, avionics.pos_d_m],
            vel_ned: [avionics.vel_n_mps, avionics.vel_e_mps, avionics.vel_d_mps],
            arm_state: 0,
        },
        valid_flags: avionics.valid_flags,
        quality: avionics.quality,
        attitude_stamp: raw_stamp(avionics.attitude_stamp.as_ref()),
        kinematics_stamp: raw_stamp(avionics.kinematics_stamp.as_ref()),
        estimator_status_stamp: raw_stamp(avionics.estimator_status_stamp.as_ref()),
    }
}

/// Enum codes ride the wire as open `i32`/`u32`; the feeder gates on exact
/// byte equality, so an out-of-range code becomes the fail-closed 255
/// rather than a truncated collision with a real code.
fn clamp_u8(value: i32) -> u8 {
    u8::try_from(value).unwrap_or(255)
}
