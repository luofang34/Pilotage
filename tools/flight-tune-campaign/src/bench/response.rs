//! The candidate-to-command-law mapping the bench vehicle flies.

use flight_tune::{AdapterError, Candidate};
use pilotage_control_feel::{AxisCurve, AxisDynamics, AxisResponse, NeutralBand};

use super::parameter;

/// Reads one candidate's parameters as an axis response.
///
/// # Errors
///
/// Returns [`AdapterError`] when a parameter is absent or not finite.
pub(super) fn response_from(candidate: &Candidate) -> Result<AxisResponse, AdapterError> {
    let read = |name: &str| -> Result<f64, AdapterError> {
        candidate
            .parameters()
            .get(name)
            .copied()
            .filter(|value| value.is_finite())
            .ok_or_else(|| AdapterError::new(format!("the candidate states no {name}")))
    };
    let apply_accel = read(parameter::APPLY_ACCEL)?;
    let apply_jerk = read(parameter::APPLY_JERK)?;
    let release_factor = read(parameter::RELEASE_FACTOR)?;
    let enter = read(parameter::NEUTRAL_ENTER)?;
    let center_expo = read(parameter::CENTER_EXPO)? as f32;
    Ok(AxisResponse {
        curve: AxisCurve {
            deadzone: read(parameter::DEADZONE)? as f32,
            center_expo,
            // The profile validator refuses an outer expo above the
            // center one; folding the search there keeps every sealed
            // winner a law a real profile will load. The outer blend
            // begins where the shipped law's does, so the trial's firm
            // input exercises it.
            outer_expo: (read(parameter::OUTER_EXPO)? as f32).min(center_expo),
            outer_start: 0.7,
        },
        neutral: NeutralBand {
            active_enter: enter as f32,
            // Leaving is harder than staying, or an input on the edge chatters.
            // The search sets the entry and the exit follows it, so no
            // candidate can propose a band with no hysteresis in it.
            active_exit: (enter * 0.65) as f32,
            dwell_ms: read(parameter::NEUTRAL_DWELL_MS)?.max(0.0) as u32,
        },
        dynamics: AxisDynamics {
            apply_accel: apply_accel as f32,
            apply_jerk: apply_jerk as f32,
            // Letting go is never slower than asking, whatever the search
            // proposes: a release that lagged the apply would take longer to
            // stop commanding than to start.
            release_accel: (apply_accel * release_factor.max(1.0)) as f32,
            release_jerk: (apply_jerk * release_factor.max(1.0)) as f32,
            reversal_accel: apply_accel as f32,
            reversal_jerk: apply_jerk as f32,
        },
    })
}
