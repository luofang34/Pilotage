//! The stamped signal groups a `TelemetrySample` carries, in the browser
//! ingress's field vocabulary.
//!
//! Every group here is independent: its own source identity, epoch, sequence,
//! clock, and role (ADR-0018). Each decode is fail-closed twice over — a
//! group without its provenance stamp is `None` rather than proto3 zeros
//! presented as a measurement, and a stamp whose role does not match the lane
//! exactly is `None` rather than a mislabeled lane feeding a display or a
//! control decision.

use pilotage_protocol::wire;
use serde::Serialize;

use super::{Stamp, stamp_message};

#[cfg(test)]
mod tests;

#[derive(Serialize, Clone, Copy)]
pub(super) struct Quat {
    w: f32,
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Serialize, Clone, Copy)]
pub(super) struct Attitude {
    quat: Quat,
    rates: [f32; 3],
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub(super) struct Kinematics {
    pos_ned: [f32; 3],
    vel_ned: [f32; 3],
}

/// Raw avionics estimate in the exact shape the browser ingress consumes: the
/// attitude and kinematics groups are present only when their acquisition stamp
/// is, and the flattened `quat`/`rates`/`posNed`/`velNed` mirror those groups.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Avionics {
    attitude: Option<Attitude>,
    kinematics: Option<Kinematics>,
    quat: Option<Quat>,
    rates: Option<[f32; 3]>,
    pos_ned: Option<[f32; 3]>,
    vel_ned: Option<[f32; 3]>,
    valid_flags: u32,
    quality: u32,
    arm_state: u32,
    attitude_stamp: Option<Stamp>,
    kinematics_stamp: Option<Stamp>,
    estimator_status_stamp: Option<Stamp>,
    /// Pressure altitude (ISA standard datum), meters; `None` without
    /// its stamp — an absent measurement is never a zero reading.
    baro_alt_m: Option<f32>,
    baro_stamp: Option<Stamp>,
}

/// Simulation-truth oracle sample in the browser's shape: the simulator's
/// pose under its own provenance stamp. Kept structurally apart from
/// `Avionics` — truth is never merged into the estimate the panels consume.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SimTruth {
    quat: Quat,
    pos_ned: [f32; 3],
    vel_ned: [f32; 3],
    valid_flags: u32,
    geodetic: Option<GeodeticFix>,
    stamp: Stamp,
}

/// Where a vehicle is on the Earth, with the datum the position is measured
/// against fully declared (ADR-0022).
///
/// The browser receives this only when the geodetic contract accepts it, so
/// a datum this build cannot interpret reaches a reader as no position at
/// all rather than as a place drawn on a map.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeodeticFix {
    latitude_deg: f64,
    longitude_deg: f64,
    height_m: f64,
    horizontal_datum: u32,
    realization: u32,
    vertical_datum: u32,
    geoid_model: u32,
    terrain_ref: u32,
    baro_setting: u32,
    local_origin: u64,
    /// 1-sigma accuracy in millimetres, or `None` when the producer stated
    /// none. Zero on the wire is UNSTATED, never perfect: proto3 omits a
    /// zero, so silence and a perfection claim are the same bytes.
    horizontal_accuracy_mm: Option<u32>,
    vertical_accuracy_mm: Option<u32>,
}

/// Reads a fix through the geodetic contract, so the refusals are the
/// contract's own: an unknown datum, a missing realization, an MSL height
/// that names no geoid, or a latitude past the pole is no fix.
fn geodetic_message(fix: wire::GeodeticFix) -> Option<GeodeticFix> {
    use pilotage_geo::{
        BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
        LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
    };

    let vertical = VerticalPosition::new(
        fix.height_m,
        VerticalDatum::from_u8(u8::try_from(fix.vertical_datum).ok()?)?,
        GeoidModelId(u16::try_from(fix.geoid_model).ok()?),
        TerrainRefId(fix.terrain_ref),
        BaroSettingId(fix.baro_setting),
        LocalOriginId(fix.local_origin),
    )
    .ok()?;
    let position = GeodeticPosition::new(
        fix.latitude_deg,
        fix.longitude_deg,
        HorizontalDatum::from_u8(u8::try_from(fix.horizontal_datum).ok()?)?,
        DatumRealizationId(u16::try_from(fix.realization).ok()?),
        vertical,
    )
    .ok()?;
    let accuracy = |mm: u32| (mm > 0).then_some(mm);
    Some(GeodeticFix {
        latitude_deg: position.latitude_deg,
        longitude_deg: position.longitude_deg,
        height_m: position.vertical.height_m,
        horizontal_datum: u32::from(position.horizontal_datum.to_u8()),
        realization: u32::from(position.realization.0),
        vertical_datum: u32::from(position.vertical.datum.to_u8()),
        geoid_model: u32::from(position.vertical.geoid.0),
        terrain_ref: position.vertical.terrain_ref.0,
        baro_setting: position.vertical.baro_setting.0,
        local_origin: position.vertical.origin.0,
        horizontal_accuracy_mm: accuracy(fix.horizontal_accuracy_mm),
        vertical_accuracy_mm: accuracy(fix.vertical_accuracy_mm),
    })
}

/// FC-owned arm/mode state under its own provenance stamp.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FcState {
    arm_state: u32,
    stamp: Stamp,
}

/// Gimbal payload-device orientation under its own provenance stamp.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Gimbal {
    quat: Quat,
    rates_rad_s: [f32; 3],
    flags: u32,
    failure_flags: u32,
    stamp: Stamp,
}

/// Active-leg navigation guidance in raw canonical units (ADR-0031): meters
/// and radians, never dots. Scaling deviations to an instrument's dot scale
/// is the client display profile's job, so the same sample can drive panels
/// with different full-scale deflection.
///
/// `lateralDeviationM` is NaN when the guidance tracks no lateral course and
/// `verticalDeviationM` is NaN without a vertical constraint; both reach JS
/// as NaN so a consumer removes that deviation rather than centering it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NavGuidance {
    to_ident: String,
    from_ident: String,
    course_rad: f32,
    lateral_deviation_m: f32,
    vertical_deviation_m: f32,
    distance_to_waypoint_m: f32,
    leg_index: u32,
    waypoint_count: u32,
    solution_quality: u32,
    stamp: Stamp,
}

/// `None` when the wire sample carries no provenance stamp: truth without
/// identity is unconsumable and is dropped, never defaulted.
pub(super) fn sim_truth_message(state: wire::SimTruthState) -> Option<SimTruth> {
    let stamp = state.stamp?;
    // Exact-role gate: a truth lane whose stamp does not carry the
    // simulation-truth role is mislabeled and unconsumable.
    if stamp.role != wire::SourceRole::SimulationTruth as i32 {
        return None;
    }
    Some(SimTruth {
        quat: Quat {
            w: state.quat_w,
            x: state.quat_x,
            y: state.quat_y,
            z: state.quat_z,
        },
        pos_ned: [state.pos_n_m, state.pos_e_m, state.pos_d_m],
        vel_ned: [state.vel_n_mps, state.vel_e_mps, state.vel_d_mps],
        valid_flags: state.valid_flags,
        // The oracle's position on the Earth, from the same observation as
        // the frame above, so it rides this sample's stamp.
        geodetic: state.geodetic.and_then(geodetic_message),
        stamp: stamp_message(stamp),
    })
}

/// `None` when the wire report carries no provenance stamp: an unstamped
/// arm state is exactly what this lane exists to prevent.
pub(super) fn fc_state_message(state: wire::FcState) -> Option<FcState> {
    let stamp = state.stamp?;
    // Exact-role gate: FC state must carry the FC-state role.
    if stamp.role != wire::SourceRole::FcState as i32 {
        return None;
    }
    Some(FcState {
        arm_state: state.arm_state,
        stamp: stamp_message(stamp),
    })
}

/// `None` without a payload-device stamp, so a mislabeled lane can never
/// point the camera view or be read as vehicle attitude.
pub(super) fn gimbal_message(state: wire::GimbalAttitude) -> Option<Gimbal> {
    let stamp = state.stamp?;
    // Exact-role gate: the gimbal lane must carry the payload-device role.
    if stamp.role != wire::SourceRole::PayloadDevice as i32 {
        return None;
    }
    Some(Gimbal {
        quat: Quat {
            w: state.quat_w,
            x: state.quat_x,
            y: state.quat_y,
            z: state.quat_z,
        },
        rates_rad_s: [state.rate_x_rad_s, state.rate_y_rad_s, state.rate_z_rad_s],
        flags: state.flags,
        failure_flags: state.failure_flags,
        stamp: stamp_message(stamp),
    })
}

/// `None` without a navigation-solution stamp: guidance is display context,
/// and a lane wearing another role is not the navigation component's
/// solution — it is never accepted as a substitute for one.
pub(super) fn nav_guidance_message(state: wire::NavGuidanceState) -> Option<NavGuidance> {
    let stamp = state.stamp?;
    // Exact-role gate: guidance must carry the navigation-solution role.
    if stamp.role != wire::SourceRole::NavigationSolution as i32 {
        return None;
    }
    Some(NavGuidance {
        to_ident: state.to_ident,
        from_ident: state.from_ident,
        course_rad: state.course_rad,
        lateral_deviation_m: state.lateral_deviation_m,
        vertical_deviation_m: state.vertical_deviation_m,
        distance_to_waypoint_m: state.distance_to_waypoint_m,
        leg_index: state.leg_index,
        waypoint_count: state.waypoint_count,
        solution_quality: state.solution_quality,
        stamp: stamp_message(stamp),
    })
}

// Surfaces the deprecated wire lane `arm_state` verbatim (hosts leave it 0);
// consumers take arm from the stamped `fcState` message instead.
#[allow(deprecated)]
pub(super) fn avionics_message(state: wire::AvionicsState) -> Avionics {
    let attitude_stamp = state.attitude_stamp.map(stamp_message);
    let kinematics_stamp = state.kinematics_stamp.map(stamp_message);
    let estimator_status_stamp = state.estimator_status_stamp.map(stamp_message);
    let baro_stamp = state.baro_stamp.map(stamp_message);
    // A group's values are meaningful only when its acquisition stamp is
    // present; absent that, the group is null, never proto3 zero displayed as a
    // measurement (ADR-0018).
    let attitude = attitude_stamp.as_ref().map(|_| Attitude {
        quat: Quat {
            w: state.quat_w,
            x: state.quat_x,
            y: state.quat_y,
            z: state.quat_z,
        },
        rates: [state.rate_p_rad_s, state.rate_q_rad_s, state.rate_r_rad_s],
    });
    let kinematics = kinematics_stamp.as_ref().map(|_| Kinematics {
        pos_ned: [state.pos_n_m, state.pos_e_m, state.pos_d_m],
        vel_ned: [state.vel_n_mps, state.vel_e_mps, state.vel_d_mps],
    });
    Avionics {
        quat: attitude.map(|attitude| attitude.quat),
        rates: attitude.map(|attitude| attitude.rates),
        pos_ned: kinematics.map(|kinematics| kinematics.pos_ned),
        vel_ned: kinematics.map(|kinematics| kinematics.vel_ned),
        baro_alt_m: baro_stamp.as_ref().map(|_| state.baro_alt_m),
        baro_stamp,
        attitude,
        kinematics,
        valid_flags: state.valid_flags,
        quality: state.quality,
        arm_state: state.arm_state,
        attitude_stamp,
        kinematics_stamp,
        estimator_status_stamp,
    }
}
