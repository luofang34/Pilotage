//! The dynamic alignment-error bound: the total angular registration error of a
//! conformal fix, combined from named, individually sourced contributors.
//!
//! Nothing here is a hidden constant. Every contributor is a field with a stated
//! formula and origin, and the total is a conservative worst-case linear sum (not
//! root-sum-square) so a consumer never under-counts. In particular there is no
//! baked-in latency offset: the latency contribution is the caller's *measured*
//! pipeline latency plus the attitude/position co-timing skew computed from the
//! stamps.
//!
//! Angular rate and speed are not standalone additive terms — they are the
//! *sensitivities* that turn a timing error into an angular error: attitude smears
//! at the body angular rate, and a near-field feature's parallax smears at
//! `speed / reference_range`. Each timing error (clock, latency, extrapolation) is
//! multiplied by that combined sensitivity, so a fast maneuver or a fast closure
//! inflates exactly the timing terms it should.

use super::interp::Interpolated;
use super::policy::ConformalPolicy;

/// The named contributors to a conformal fix's total angular alignment error,
/// radians. The total is the worst-case (linear) sum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignmentErrorBound {
    /// Calibration residual: the referenced calibration artifact's published
    /// static angular alignment bound, resolved and supplied by the caller. This
    /// crate references the calibration; it does not re-derive its budget.
    pub calibration_rad: f64,
    /// The interpolated attitude estimate's own 1-sigma angular accuracy (from
    /// the coherent snapshots' attitude quality).
    pub attitude_quality_rad: f64,
    /// Position error as an angular parallax bound at the policy reference range:
    /// `position_sigma / reference_range`.
    pub position_rad: f64,
    /// Velocity error as an angular parallax bound: the velocity 1-sigma
    /// propagated over the total timing uncertainty into a position error, then
    /// divided by the reference range — `velocity_sigma × (clock + latency +
    /// extrapolation) / reference_range`. The state is bridged across those timing
    /// gaps by the velocity, so an uncertain velocity leaves the extrapolated
    /// position (and thus the near-field registration) uncertain. This is the
    /// velocity-accuracy analog of [`Self::position_rad`].
    pub velocity_rad: f64,
    /// Clock uncertainty times the registration sensitivity: the capture-clock
    /// mapping error bound × `(angular_rate + speed / reference_range)`.
    pub clock_rad: f64,
    /// Latency times the registration sensitivity: `(measured pipeline latency +
    /// attitude/position skew)` × `(angular_rate + speed / reference_range)`.
    pub latency_rad: f64,
    /// Extrapolation times the registration sensitivity: the distance the capture
    /// time falls outside the bracket × `(angular_rate + speed / reference_range)`;
    /// zero for a true interpolation.
    pub extrapolation_rad: f64,
    /// The worst-case linear sum of the contributors — the single number the
    /// state machine compares against the policy thresholds.
    pub total_rad: f64,
}

impl AlignmentErrorBound {
    /// Whether the total is within `limit_rad`, failing closed: a non-finite
    /// total (from a non-finite input) is never within any limit, so it can never
    /// be drawn as a plausible registered scene.
    #[must_use]
    pub fn within(&self, limit_rad: f64) -> bool {
        self.total_rad.is_finite() && self.total_rad <= limit_rad
    }
}

fn ns_to_s(ns: u64) -> f64 {
    ns as f64 / 1e9
}

fn norm3_f64(v: [f64; 3]) -> f64 {
    libm::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

fn norm3_f32(v: [f32; 3]) -> f64 {
    let (x, y, z) = (f64::from(v[0]), f64::from(v[1]), f64::from(v[2]));
    libm::sqrt(x * x + y * y + z * z)
}

/// Combines the calibration residual, clock uncertainty, measured latency,
/// angular rate, position/velocity error, and extrapolation of `interp` into the
/// total alignment-error bound under `policy`.
pub(crate) fn compute(
    calibration_bound_rad: f64,
    clock_error_ns: u64,
    pipeline_latency_ns: u64,
    interp: &Interpolated,
    policy: &ConformalPolicy,
) -> AlignmentErrorBound {
    let range = policy.reference_range_m();
    let omega = norm3_f32(interp.body_rate_rps);
    let speed = norm3_f64(interp.velocity_ned_mps);
    // Radians of registration error per second of timing error: the attitude
    // channel smears at the body rate, the near-field parallax channel at
    // speed/range.
    let sensitivity = omega + speed / range;

    let clock_s = ns_to_s(clock_error_ns);
    let latency_s = ns_to_s(pipeline_latency_ns.saturating_add(interp.timing.skew_ns));
    let extrap_s = ns_to_s(interp.timing.extrapolation_ns);

    let calibration_rad = calibration_bound_rad;
    let attitude_quality_rad = interp.attitude_sigma_rad;
    let position_rad = interp.position_sigma_m / range;
    // The velocity bridges the state across the timing gaps (clock, latency,
    // extrapolation), so a velocity 1-sigma leaves the extrapolated position
    // uncertain by `velocity_sigma × timing`; as a parallax angle at the
    // reference range that is the velocity-accuracy contribution.
    let velocity_rad = interp.velocity_sigma_mps * (clock_s + latency_s + extrap_s) / range;
    let clock_rad = clock_s * sensitivity;
    let latency_rad = latency_s * sensitivity;
    let extrapolation_rad = extrap_s * sensitivity;

    AlignmentErrorBound {
        calibration_rad,
        attitude_quality_rad,
        position_rad,
        velocity_rad,
        clock_rad,
        latency_rad,
        extrapolation_rad,
        total_rad: calibration_rad
            + attitude_quality_rad
            + position_rad
            + velocity_rad
            + clock_rad
            + latency_rad
            + extrapolation_rad,
    }
}
