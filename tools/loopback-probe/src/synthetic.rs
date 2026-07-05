//! Synthetic sine-wave control payload generator (the default control
//! source when `--hid` is not given).
//!
//! Unlike the HID path, this produces a [`ControlPayload`] directly in the
//! canonical `[-1.0, 1.0]` axis convention: there is no physical device to
//! calibrate away from, so running it through `pilotage-input`'s
//! HID-raw-units normalization pipeline would be the wrong stage boundary.

use std::f32::consts::PI;
use std::time::Duration;

use pilotage_input::{axis_id_for_name, button_id_for_name};
use pilotage_protocol::{ButtonEdge, ControlPayload};

/// Steering (`roll`) sine frequency in Hz.
const STEERING_HZ: f32 = 0.2;
/// Throttle sine frequency in Hz.
const THROTTLE_HZ: f32 = 0.1;

/// Builds a [`ControlPayload`] carrying a sine-wave throttle and steering
/// pair for `elapsed` time into the run, plus a `button0` press/release
/// edge toggled once every 2 seconds so the fixed frame-rejection probe
/// (see `main`) has real edge traffic to compare against on a live host.
///
/// Steering is emitted on the `yaw` axis, not `roll`: the reference adapter's
/// `vehicle.motion` scope exposes exactly `throttle` and `yaw`, and rejects a
/// frame carrying any other axis id (`UnknownAxis`). Sending `roll` here would
/// make every synthetic frame reject at the adapter and the skiff would never
/// move.
///
/// # Errors
///
/// Returns an error only if the well-known logical-name table in
/// `pilotage-input` ever stops recognizing `yaw`/`throttle`/`button0`,
/// which would indicate a crate version skew this binary cannot recover
/// from at runtime.
pub fn payload_at(
    elapsed: Duration,
    edge: Option<ButtonEdge>,
) -> Result<ControlPayload, pilotage_input::ProfileError> {
    let t = elapsed.as_secs_f32();
    let steering = (t * 2.0 * PI * STEERING_HZ).sin();
    let throttle = (t * 2.0 * PI * THROTTLE_HZ).sin();
    let mut axes = vec![
        (axis_id_for_name("yaw")?, steering),
        (axis_id_for_name("throttle")?, throttle),
    ];
    axes.sort_by_key(|(id, _)| *id);
    let mut edges = Vec::new();
    if let Some(edge) = edge {
        edges.push((button_id_for_name("button0")?, edge));
    }
    Ok(ControlPayload { axes, edges })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::payload_at;
    use std::time::Duration;

    #[test]
    fn payload_at_zero_is_near_neutral() {
        let payload = payload_at(Duration::from_secs(0), None).expect("builds");
        for (_, value) in &payload.axes {
            assert!(value.abs() < 1e-6);
        }
        assert!(payload.edges.is_empty());
    }

    #[test]
    fn payload_varies_over_time() {
        let early = payload_at(Duration::from_millis(0), None).expect("builds");
        let later = payload_at(Duration::from_millis(500), None).expect("builds");
        assert_ne!(early.axes, later.axes);
    }

    #[test]
    fn payload_carries_supplied_edge() {
        use pilotage_protocol::ButtonEdge;
        let payload =
            payload_at(Duration::from_secs(0), Some(ButtonEdge::Pressed)).expect("builds");
        assert_eq!(payload.edges.len(), 1);
        assert_eq!(payload.edges[0].1, ButtonEdge::Pressed);
    }
}
