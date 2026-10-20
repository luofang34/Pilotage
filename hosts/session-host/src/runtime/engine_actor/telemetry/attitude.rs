//! Mutually exclusive wire representations for aircraft attitude.

use pilotage_adapter_api::AvionicsAttitudeSample;
use pilotage_protocol::wire;

use super::{ESTIMATOR_QUALITY_UNUSABLE, measurement_stamp_to_wire};

#[cfg(test)]
mod tests;

pub(super) fn apply(sample: Option<AvionicsAttitudeSample>, state: &mut wire::AvionicsState) {
    match sample {
        Some(AvionicsAttitudeSample::Canonical {
            quat_wxyz: [w, x, y, z],
            rates_rps,
            stamp,
        }) => {
            state.quat_w = w;
            state.quat_x = x;
            state.quat_y = y;
            state.quat_z = z;
            state.attitude_stamp = Some(measurement_stamp_to_wire(stamp));
            if let Some([p, q, r]) = rates_rps.filter(|rates| rates.iter().all(|v| v.is_finite())) {
                state.rate_p_rad_s = p;
                state.rate_q_rad_s = q;
                state.rate_r_rad_s = r;
            } else {
                state.valid_flags &= !0b0010;
            }
        }
        Some(AvionicsAttitudeSample::ReportedEuler {
            pitch_rad,
            roll_rad,
            yaw_rad,
            stamp,
        }) => {
            // A status report cannot assign a known frame to these values.
            state.valid_flags &= !0b0011;
            if state.valid_flags == 0 {
                state.quality = ESTIMATOR_QUALITY_UNUSABLE;
            }
            if [pitch_rad, roll_rad, yaw_rad]
                .iter()
                .all(|value| value.is_finite())
            {
                state.reported_euler = Some(wire::ReportedEulerAttitude {
                    pitch_rad,
                    roll_rad,
                    yaw_rad,
                    stamp: Some(measurement_stamp_to_wire(stamp)),
                });
            }
        }
        None => {}
    }
}
