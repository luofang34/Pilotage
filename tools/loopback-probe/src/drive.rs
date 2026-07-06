//! `--drive` scripted control pattern: forward for the first half of the run,
//! then an arcing turn for the second half, so a real vehicle (Gazebo adapter)
//! visibly moves and turns rather than sitting still or oscillating in place
//! like the default synthetic sine generator.
//!
//! Unlike `synthetic::payload_at`, this is a one-shot script keyed on elapsed
//! run time and the run's total budget, not a continuous waveform: the demo
//! goal is "prove the real vehicle moved", which reads more clearly from two
//! distinct phases than from a sine sweep.

use std::time::Duration;

use pilotage_input::axis_id_for_name;
use pilotage_protocol::ControlPayload;

use crate::error::ProbeError;

/// Forward throttle command during the drive script's first phase, in the
/// canonical `[-1.0, 1.0]` axis convention.
const DRIVE_THROTTLE: f32 = 0.6;
/// Yaw command during the drive script's arc phase.
const DRIVE_YAW: f32 = 0.5;
/// Reduced throttle during the arc phase so the turn is visibly an arc, not a
/// straight line with added spin.
const ARC_THROTTLE: f32 = 0.35;

/// Builds the `--drive` script's [`ControlPayload`] for `elapsed` time into a
/// run budgeted for `total`: forward-only for the first half, then a
/// forward-arcing turn for the second half.
///
/// # Errors
///
/// Returns an error only if `pilotage-input`'s well-known logical-name table
/// ever stops recognizing `throttle`/`yaw`, indicating a crate version skew
/// this binary cannot recover from at runtime.
pub fn payload_at(elapsed: Duration, total: Duration) -> Result<ControlPayload, ProbeError> {
    let halfway = total / 2;
    let (throttle, yaw) = if elapsed < halfway {
        (DRIVE_THROTTLE, 0.0)
    } else {
        (ARC_THROTTLE, DRIVE_YAW)
    };
    let mut axes = vec![
        (axis_id_for_name("throttle")?, throttle),
        (axis_id_for_name("yaw")?, yaw),
    ];
    axes.sort_by_key(|(id, _)| *id);
    Ok(ControlPayload {
        axes,
        edges: Vec::new(),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::payload_at;
    use std::time::Duration;

    #[test]
    fn first_half_drives_forward_only() {
        let total = Duration::from_secs(10);
        let payload = payload_at(Duration::from_secs(1), total).expect("builds");
        for (axis, value) in &payload.axes {
            if axis.as_u16() == 2 {
                assert!(*value > 0.0, "throttle should be positive");
            } else if axis.as_u16() == 3 {
                assert_eq!(*value, 0.0, "yaw should be neutral in the forward phase");
            }
        }
    }

    #[test]
    fn second_half_arcs() {
        let total = Duration::from_secs(10);
        let payload = payload_at(Duration::from_secs(6), total).expect("builds");
        for (axis, value) in &payload.axes {
            if axis.as_u16() == 3 {
                assert!(*value > 0.0, "yaw should be nonzero in the arc phase");
            }
        }
    }
}
