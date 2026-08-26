//! Where the vehicle says it is, read off one telemetry lane.
//!
//! The situation session ingests surveillance, weather and terrain. A vehicle
//! under this operator's own control appears in none of them, so its position
//! reaches the map through this and nothing else.
//!
//! The rules here are the browser's rules, deliberately. Both clients draw the
//! same mark from the same measurements, and a rule stated twice is a rule
//! that drifts — so the thresholds and the lane choice are the ones the shared
//! corpus pins, and any change to them has to move the corpus first.

use pilotage_protocol::wire;

/// Below this speed a velocity states no direction worth drawing: the track
/// is noise around a stationary vehicle rather than a course.
const TRACK_FLOOR_MPS: f64 = 0.5;

/// Bit 0 of the validity mask: the lane states an attitude.
const VALID_ATTITUDE: u32 = 1 << 0;
/// Bit 3: the lane states a velocity.
const VALID_VELOCITY: u32 = 1 << 3;

/// A vehicle's position and the directions that ride with it.
pub(super) struct VehicleFix {
    pub(super) latitude_deg: f64,
    pub(super) longitude_deg: f64,
    pub(super) heading_deg: Option<f64>,
    pub(super) course_deg: Option<f64>,
    pub(super) ground_speed_mps: Option<f64>,
    pub(super) from_simulator: bool,
}

/// Reads one sample into a fix, or nothing when no lane states a position.
///
/// The oracle wins where a session has one, because a session that has one is
/// being judged against it. A session without one is the normal case, not a
/// failure, and it is the only case a physical vehicle has.
pub(super) fn from_sample(sample: &wire::TelemetrySample) -> Option<VehicleFix> {
    if let Some(truth) = sample.sim_truth.as_ref()
        && let Some(geodetic) = truth.geodetic.as_ref()
        // Truth without provenance is discarded, the same refusal the
        // decoder makes: a position that cannot be shown to be this
        // vehicle's, this boot's and this moment's is not drawn.
        && truth.stamp.is_some()
    {
        return Some(VehicleFix {
            latitude_deg: geodetic.latitude_deg,
            longitude_deg: geodetic.longitude_deg,
            heading_deg: heading_from(
                [truth.quat_w, truth.quat_x, truth.quat_y, truth.quat_z],
                truth.valid_flags,
            ),
            course_deg: track_from([truth.vel_n_mps, truth.vel_e_mps], truth.valid_flags)
                .map(|(bearing, _)| bearing),
            ground_speed_mps: track_from([truth.vel_n_mps, truth.vel_e_mps], truth.valid_flags)
                .map(|(_, speed)| speed),
            from_simulator: true,
        });
    }

    let avionics = sample.avionics.as_ref()?;
    let geodetic = avionics.geodetic.as_ref()?;
    avionics.geodetic_stamp.as_ref()?;
    // On the estimate lane the mask is a latched authorization from the
    // estimator, and it means nothing without the status observation backing
    // it. Absence is no authorization, and a consumer of it fails closed.
    let flags = if avionics.estimator_status_stamp.is_some() {
        avionics.valid_flags
    } else {
        0
    };
    Some(VehicleFix {
        latitude_deg: geodetic.latitude_deg,
        longitude_deg: geodetic.longitude_deg,
        heading_deg: heading_from(
            [
                avionics.quat_w,
                avionics.quat_x,
                avionics.quat_y,
                avionics.quat_z,
            ],
            flags,
        ),
        course_deg: track_from([avionics.vel_n_mps, avionics.vel_e_mps], flags)
            .map(|(bearing, _)| bearing),
        ground_speed_mps: track_from([avionics.vel_n_mps, avionics.vel_e_mps], flags)
            .map(|(_, speed)| speed),
        from_simulator: false,
    })
}

/// Where the nose points, from the attitude quaternion.
///
/// The full form rather than `1 - 2(y² + z²)`: it is exact at any scale, and a
/// quaternion that arrived slightly off unit length still yields the angle it
/// represents instead of a silently wrong one.
fn heading_from(quat: [f32; 4], valid_flags: u32) -> Option<f64> {
    if valid_flags & VALID_ATTITUDE == 0 {
        return None;
    }
    let [w, x, y, z] = quat.map(f64::from);
    let norm_squared = w * w + x * x + y * y + z * z;
    // A quaternion nowhere near unit length is not an attitude that was
    // measured; it is a field nobody filled in.
    if !(0.9..=1.1).contains(&norm_squared) {
        return None;
    }
    let yaw = f64::atan2(
        2.0 * z.mul_add(w, x * y),
        z.mul_add(-z, y.mul_add(-y, x.mul_add(x, w * w))),
    );
    Some(wrap_bearing(yaw.to_degrees()))
}

/// Track over the ground and the speed along it.
fn track_from(vel_ne: [f32; 2], valid_flags: u32) -> Option<(f64, f64)> {
    if valid_flags & VALID_VELOCITY == 0 {
        return None;
    }
    let [north, east] = vel_ne.map(f64::from);
    if !north.is_finite() || !east.is_finite() {
        return None;
    }
    let speed = north.hypot(east);
    if speed < TRACK_FLOOR_MPS {
        return None;
    }
    Some((wrap_bearing(f64::atan2(east, north).to_degrees()), speed))
}

/// A bearing in `[0, 360)`.
fn wrap_bearing(degrees: f64) -> f64 {
    degrees.rem_euclid(360.0)
}

#[cfg(test)]
mod tests;
