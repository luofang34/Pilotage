//! Calibration identity, lifecycle, and provenance.
//!
//! A calibration artifact is traceable: it carries a stable id, a content
//! version, the version of the tool that produced it, an effective validity
//! window, its provenance, and the residual error of the fit. The
//! [`CalibrationId`] itself is the routing identity a video frame already
//! carries (HUD-01 identity contract); the rest pins what that id resolves to.

pub use crate::video::CalibrationId;

/// The content version of a calibration: a change in any calibrated value bumps
/// this. Distinct from the schema version, which is fixed by the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CalibrationVersion(pub u32);

/// The version of the calibration tool that produced an artifact, so a fit can
/// be traced to the exact tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolVersion {
    /// Tool major version.
    pub major: u16,
    /// Tool minor version.
    pub minor: u16,
    /// Tool patch version.
    pub patch: u16,
}

/// The window during which a calibration is effective, in Unix (UTC)
/// nanoseconds. Sans-IO: expiry is judged against a caller-supplied `now`,
/// never a read of the system clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectivePeriod {
    /// Inclusive start, Unix epoch nanoseconds (UTC).
    pub start_unix_ns: u64,
    /// Exclusive end, Unix epoch nanoseconds (UTC).
    pub end_unix_ns: u64,
}

impl EffectivePeriod {
    /// Whether `now_unix_ns` falls in `[start, end)`. Fails closed for an
    /// inverted window (`end <= start`): such a period is never effective.
    #[must_use]
    pub fn contains(&self, now_unix_ns: u64) -> bool {
        self.start_unix_ns < self.end_unix_ns
            && now_unix_ns >= self.start_unix_ns
            && now_unix_ns < self.end_unix_ns
    }
}

/// Where a calibration came from. An enum rather than free text so it hashes to
/// a fixed-width field and cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ProvenanceSource {
    /// Produced by the deterministic simulator calibration tool from synthetic
    /// targets (never a measurement of real optics).
    SimSyntheticTool = 1,
}

/// The residual reprojection error of a calibration fit, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Residuals {
    /// Root-mean-square reprojection residual, in pixels.
    pub rms_px: f64,
    /// Maximum reprojection residual, in pixels.
    pub max_px: f64,
}

/// Whether a calibration may be used. A non-[`ValidityStatus::Valid`] status
/// keeps conformal output closed regardless of the other fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ValidityStatus {
    /// Provisional: produced but not accepted for use.
    Provisional = 0,
    /// Valid: accepted for SIM conformal projection.
    Valid = 1,
    /// Revoked: withdrawn; must not be used.
    Revoked = 2,
}

/// The identity and lifecycle metadata of one calibration artifact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationIdentity {
    /// Routing identity a video frame carries.
    pub calibration_id: CalibrationId,
    /// Physical camera this calibration describes.
    pub camera_id: u32,
    /// Content version.
    pub version: CalibrationVersion,
    /// Producing tool version.
    pub tool_version: ToolVersion,
    /// Effective validity window.
    pub effective: EffectivePeriod,
    /// Provenance.
    pub provenance: ProvenanceSource,
    /// Residual reprojection error of the fit.
    pub residuals: Residuals,
    /// Whether the calibration may be used.
    pub status: ValidityStatus,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::EffectivePeriod;

    #[test]
    fn effective_period_contains_within_window() {
        let period = EffectivePeriod {
            start_unix_ns: 100,
            end_unix_ns: 200,
        };
        assert!(period.contains(100), "inclusive start");
        assert!(period.contains(150));
        assert!(!period.contains(200), "exclusive end");
        assert!(!period.contains(50), "before start");
    }

    #[test]
    fn inverted_period_is_never_effective() {
        let inverted = EffectivePeriod {
            start_unix_ns: 200,
            end_unix_ns: 100,
        };
        assert!(!inverted.contains(150), "an inverted window fails closed");
    }
}
