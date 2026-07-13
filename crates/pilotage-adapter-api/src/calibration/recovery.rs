//! Deterministic simulator calibration tool: recover known pinhole intrinsics
//! from synthetic targets and report the residuals (ADR-0021).
//!
//! This is the SIM verification that a calibration fit is self-consistent, not
//! a measurement of real optics. It generates a fixed grid of camera-frame
//! target points, projects them through a known pinhole model, quantizes each
//! projection to whole pixels (the only error source, so the run is fully
//! deterministic and free of wall-clock flakiness), then recovers the focal
//! lengths and principal point by linear least squares and records the
//! reprojection residuals. Extrinsics and distortion are declared from the sim
//! world, not recovered here; this tool verifies the intrinsics.

use super::geometry::{OpticalConvention, PinholeIntrinsics};

/// Documented tolerance: a recovered focal length must be within this fraction
/// of the known value. Generous relative to the sub-pixel quantization error
/// so the bound is a guardrail, not a knife-edge.
pub const FOCAL_TOLERANCE_RATIO: f64 = 0.01;

/// Documented tolerance: a recovered principal-point coordinate must be within
/// this many pixels of the known value.
pub const PRINCIPAL_TOLERANCE_PX: f64 = 1.0;

/// Documented tolerance: the reprojection residual RMS must stay under this
/// many pixels (the projections are quantized to whole pixels).
pub const RESIDUAL_RMS_TOLERANCE_PX: f64 = 0.5;

/// One synthetic target: a known camera-frame point and its quantized pixel
/// projection through the known model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyntheticTarget {
    /// Camera-frame point `(X, Y, Z)` with `Z > 0`, in meters.
    pub point_camera_m: [f64; 3],
    /// Observed pixel `(u, v)`, quantized to whole pixels.
    pub observed_px: [f64; 2],
}

/// The result of a recovery run: the recovered intrinsics, the reprojection
/// residuals, and the recovered-vs-known errors. This is the verification
/// report artifact's content.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecoveryReport {
    /// Number of synthetic targets used.
    pub target_count: usize,
    /// Recovered intrinsics.
    pub recovered: PinholeIntrinsics,
    /// Root-mean-square reprojection residual, in pixels.
    pub residual_rms_px: f64,
    /// Maximum reprojection residual, in pixels.
    pub residual_max_px: f64,
    /// `|recovered - known| / known` for the `x` focal length.
    pub focal_x_error_ratio: f64,
    /// `|recovered - known| / known` for the `y` focal length.
    pub focal_y_error_ratio: f64,
    /// `|recovered - known|` for the principal `x`, in pixels.
    pub principal_x_error_px: f64,
    /// `|recovered - known|` for the principal `y`, in pixels.
    pub principal_y_error_px: f64,
}

impl RecoveryReport {
    /// Whether every recovered parameter and the residual RMS fall within the
    /// documented tolerances.
    #[must_use]
    pub fn within_tolerance(&self) -> bool {
        self.focal_x_error_ratio <= FOCAL_TOLERANCE_RATIO
            && self.focal_y_error_ratio <= FOCAL_TOLERANCE_RATIO
            && self.principal_x_error_px <= PRINCIPAL_TOLERANCE_PX
            && self.principal_y_error_px <= PRINCIPAL_TOLERANCE_PX
            && self.residual_rms_px <= RESIDUAL_RMS_TOLERANCE_PX
    }
}

/// Generates a fixed grid of synthetic targets and their quantized projections
/// through `known`. The grid spans the image via ratios `X/Z` and `Y/Z`, at
/// `Z = 1 m`; quantization to whole pixels is the sole (bounded) error.
#[must_use]
pub fn synthetic_targets(known: &PinholeIntrinsics) -> Vec<SyntheticTarget> {
    let mut targets = Vec::new();
    let steps = 12;
    for i in 0..=steps {
        for j in 0..=steps {
            let rx = -0.6 + 1.2 * (f64::from(i) / f64::from(steps));
            let ry = -0.45 + 0.9 * (f64::from(j) / f64::from(steps));
            let u = (known.focal_x_px * rx + known.principal_x_px).round();
            let v = (known.focal_y_px * ry + known.principal_y_px).round();
            targets.push(SyntheticTarget {
                point_camera_m: [rx, ry, 1.0],
                observed_px: [u, v],
            });
        }
    }
    targets
}

/// Linear least-squares slope and intercept of `obs` against `ratio`.
fn fit_line(pairs: &[(f64, f64)]) -> (f64, f64) {
    let n = pairs.len() as f64;
    let sum_r: f64 = pairs.iter().map(|(r, _)| *r).sum();
    let sum_o: f64 = pairs.iter().map(|(_, o)| *o).sum();
    let sum_rr: f64 = pairs.iter().map(|(r, _)| r * r).sum();
    let sum_ro: f64 = pairs.iter().map(|(r, o)| r * o).sum();
    let slope = (n * sum_ro - sum_r * sum_o) / (n * sum_rr - sum_r * sum_r);
    let intercept = (sum_o - slope * sum_r) / n;
    (slope, intercept)
}

/// Recovers pinhole intrinsics from synthetic targets by linear least squares.
#[must_use]
pub fn recover_intrinsics(targets: &[SyntheticTarget]) -> PinholeIntrinsics {
    let x_pairs: Vec<(f64, f64)> = targets
        .iter()
        .map(|t| (t.point_camera_m[0] / t.point_camera_m[2], t.observed_px[0]))
        .collect();
    let y_pairs: Vec<(f64, f64)> = targets
        .iter()
        .map(|t| (t.point_camera_m[1] / t.point_camera_m[2], t.observed_px[1]))
        .collect();
    let (focal_x_px, principal_x_px) = fit_line(&x_pairs);
    let (focal_y_px, principal_y_px) = fit_line(&y_pairs);
    PinholeIntrinsics {
        focal_x_px,
        focal_y_px,
        principal_x_px,
        principal_y_px,
        skew_px: 0.0,
        convention: OpticalConvention::OpenCv,
    }
}

fn residuals(recovered: &PinholeIntrinsics, targets: &[SyntheticTarget]) -> (f64, f64) {
    let mut sum_sq = 0.0;
    let mut max = 0.0_f64;
    for t in targets {
        let rx = t.point_camera_m[0] / t.point_camera_m[2];
        let ry = t.point_camera_m[1] / t.point_camera_m[2];
        let du = recovered.focal_x_px * rx + recovered.principal_x_px - t.observed_px[0];
        let dv = recovered.focal_y_px * ry + recovered.principal_y_px - t.observed_px[1];
        let err = (du * du + dv * dv).sqrt();
        sum_sq += err * err;
        max = max.max(err);
    }
    ((sum_sq / targets.len() as f64).sqrt(), max)
}

/// Recovers `known` from synthetic targets and reports the residuals and the
/// recovered-vs-known errors.
#[must_use]
pub fn recovery_report(known: &PinholeIntrinsics) -> RecoveryReport {
    let targets = synthetic_targets(known);
    let recovered = recover_intrinsics(&targets);
    let (residual_rms_px, residual_max_px) = residuals(&recovered, &targets);
    RecoveryReport {
        target_count: targets.len(),
        recovered,
        residual_rms_px,
        residual_max_px,
        focal_x_error_ratio: (recovered.focal_x_px - known.focal_x_px).abs() / known.focal_x_px,
        focal_y_error_ratio: (recovered.focal_y_px - known.focal_y_px).abs() / known.focal_y_px,
        principal_x_error_px: (recovered.principal_x_px - known.principal_x_px).abs(),
        principal_y_error_px: (recovered.principal_y_px - known.principal_y_px).abs(),
    }
}

/// Runs the recovery against the published simulator FPV calibration's
/// intrinsics and returns the report.
#[must_use]
pub fn verify_sim_recovery() -> RecoveryReport {
    recovery_report(&super::sim::sim_fpv_calibration().geometry.intrinsics)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{RESIDUAL_RMS_TOLERANCE_PX, verify_sim_recovery};

    #[test]
    fn sim_recovery_is_within_documented_tolerance() {
        let report = verify_sim_recovery();
        assert!(report.target_count > 0);
        assert!(
            report.within_tolerance(),
            "recovery out of tolerance: {report:?}"
        );
        assert!(report.residual_rms_px <= RESIDUAL_RMS_TOLERANCE_PX);
    }

    #[test]
    fn recovery_is_deterministic() {
        assert_eq!(verify_sim_recovery(), verify_sim_recovery());
    }
}
