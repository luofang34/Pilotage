//! Wire-to-feeder conversions.
//!
//! The wire message is prost-generated; the feeder speaks its own typed
//! sample. The conversion is total and judgement-free: absent stamps stay
//! absent, and the feeder's admission rules — not this module — decide
//! what displays.

use indicate_instrument_feeder::RawStamp;
use indicate_instrument_feeder::avionics::{AttitudeGroup, AvionicsSample, KinematicsGroup};
use indicate_instrument_feeder::nav_guidance::Guidance;
use indicate_instrument_state::{
    IdentStr, NavFromTo, NavSequencing, PositionIntegrity, PositionSource,
};
use pilotage_instrument_runtime::feeder::nav_scale_from_wire_phase;
use pilotage_protocol::wire;

#[cfg(test)]
mod tests;

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

/// Converts one wire navigation-guidance publication into the feeder's
/// stamped sample, wire flight-phase code (`1..=4`, or `0` unstated), and
/// optional RNP in meters. Returns `None` when unstamped.
#[must_use]
pub fn nav_guidance_sample(
    state: &wire::NavGuidanceState,
) -> Option<((RawStamp, Guidance), u8, Option<f32>)> {
    let stamp = raw_stamp(state.stamp.as_ref())?;
    let phase_code = match state.flight_phase {
        1..=4 => state.flight_phase as u8,
        _ => 0,
    };
    let rnp_m = (state.rnp_m.is_finite() && state.rnp_m > 0.0).then_some(state.rnp_m);
    let sequencing = match state.sequencing {
        0 => NavSequencing::Sequencing,
        1 => NavSequencing::Hold,
        2 => NavSequencing::Suspended,
        _ => NavSequencing::Unknown,
    };
    let to_from = match state.to_from {
        0 => NavFromTo::Off,
        1 => NavFromTo::To,
        2 => NavFromTo::From,
        _ => NavFromTo::Unknown,
    };
    let position_source = match u8::try_from(state.position_source) {
        Ok(code @ 1..=5) => PositionSource::from_u8(code),
        _ => PositionSource::Unknown,
    };
    let position_integrity = match state.position_integrity {
        1 => PositionIntegrity::Normal,
        2 => PositionIntegrity::Degraded,
        3 => PositionIntegrity::SensorExcluded,
        _ => PositionIntegrity::Unknown,
    };
    Some((
        (
            stamp,
            Guidance {
                to_ident: IdentStr::new(&state.to_ident).unwrap_or(IdentStr::INVALID),
                from_ident: IdentStr::new(&state.from_ident).unwrap_or(IdentStr::INVALID),
                course_rad: state.course_rad,
                lateral_deviation_m: state.lateral_deviation_m,
                vertical_deviation_m: state.vertical_deviation_m,
                distance_to_waypoint_m: state.distance_to_waypoint_m,
                scale: nav_scale_from_wire_phase(state.flight_phase),
                to_from,
                sequencing,
                position_source,
                position_integrity,
                leg_index: state.leg_index,
                waypoint_count: state.waypoint_count,
                solution_quality: state.solution_quality,
            },
        ),
        phase_code,
        rnp_m,
    ))
}
