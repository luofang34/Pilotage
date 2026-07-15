//! The conformal-registration verdict and its fixed precedence.
//!
//! This is the conformal sibling of [`crate::SvsAvailability`]: like that
//! verdict, every non-nominal outcome carries a typed reason, and the precedence
//! is fixed so the same inputs always yield the same verdict. The verdict decides
//! whether registered symbology may be drawn:
//!
//! - [`ConformalState::Valid`] — draw the cues registered to the world;
//! - [`ConformalState::Limited`] — draw them, but marked as reduced confidence;
//! - [`ConformalState::NonConformal`] — **remove** the registered cues (the
//!   budget or a registration limit was exceeded); a consumer may show a
//!   non-conformal fallback but must never leave plausible stale alignment;
//! - [`ConformalState::Unavailable`] — no usable state or camera at all.
//!
//! Only [`ConformalState::Valid`] and [`ConformalState::Limited`] draw cues, so a
//! consumer that honors [`ConformalState::draws_cues`] can never paint a stale
//! horizon or flight-path marker after the error budget is exceeded.

use crate::availability::{AvailabilityReason, SvsAvailability};

use super::budget::AlignmentErrorBound;
use super::policy::ConformalPolicy;

/// Why a conformal fix is not fully registered. Each verdict below `Valid` names
/// the specific condition that decided it, so the outcome is traceable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConformalReason {
    /// No fault; the fix is fully registered.
    Nominal,
    /// The underlying synthetic-vision state is degraded or unavailable; carries
    /// the delegated [`AvailabilityReason`] rather than re-deriving it.
    Availability(AvailabilityReason),
    /// The projection view failed validation or is not a perspective conformal
    /// view, or the resolved view geometry failed validation, so no trustworthy
    /// camera model resolves.
    ViewInvalid,
    /// The resolved view geometry was resolved from a different calibration than
    /// the projection view references, so the projection would run through the
    /// wrong camera model.
    CalibrationMismatch,
    /// The capture time — or a kinematic value's epoch — is not on the same
    /// clock domain and time scale as the aircraft state, so it cannot be
    /// aligned to it.
    ClockIncoherent,
    /// A sample's attitude and position are not one declared coherent snapshot,
    /// so the pose is not a single trustworthy fix.
    SnapshotIncoherent,
    /// The two bracket samples are not one continuous source stream, or the
    /// bracket is out of time order (a reordered or restarted timeline).
    StreamDiscontinuity,
    /// A bracket endpoint's attitude quaternion is not a unit rotation (e.g. a
    /// zero or denormalized quaternion), so it cannot orient the projection.
    AttitudeNotARotation,
    /// A bracket endpoint's velocity or body rate is not in its required frame
    /// (velocity must be NED, body rate body-frame), so it cannot be projected.
    KinematicFrame,
    /// A bracket endpoint's velocity or body rate does not share the pose's
    /// coherent snapshot, so the kinematics are not one trustworthy fix.
    KinematicProvenance,
    /// A sample's components (pose, velocity, body rate) are co-timed worse
    /// than the policy allows.
    ExcessiveSkew,
    /// The capture time falls further outside the bracket than the policy allows.
    ExcessiveExtrapolation,
    /// The body angular rate exceeds the policy's conformal limit.
    ExcessiveRate,
    /// The dynamic alignment-error bound exceeds the policy's limit.
    ErrorBudgetExceeded,
}

/// The conformal-registration verdict. Every outcome below `Valid` carries the
/// [`ConformalReason`] that decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConformalState {
    /// Fully registered; draw the cues locked to the world.
    Valid,
    /// Registered but reduced; draw the cues marked as lower confidence.
    Limited(ConformalReason),
    /// Not registered; remove the conformal cues for the given reason.
    NonConformal(ConformalReason),
    /// No usable aircraft state or camera model, for the given reason.
    Unavailable(ConformalReason),
}

impl ConformalState {
    /// The reason behind this verdict ([`ConformalReason::Nominal`] when valid).
    #[must_use]
    pub const fn reason(self) -> ConformalReason {
        match self {
            Self::Valid => ConformalReason::Nominal,
            Self::Limited(r) | Self::NonConformal(r) | Self::Unavailable(r) => r,
        }
    }

    /// Whether registered conformal cues may be drawn (valid or limited only).
    /// A consumer that gates drawing on this can never paint a stale registration
    /// after the budget is exceeded.
    #[must_use]
    pub const fn draws_cues(self) -> bool {
        matches!(self, Self::Valid | Self::Limited(_))
    }

    /// Whether the fix is fully registered (valid only).
    #[must_use]
    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// Classifies a fix from its post-interpolation facts by fixed precedence: a
/// registration limit (skew, extrapolation, rate) removes the cues first; then an
/// exceeded error budget removes them; then a within-`limited`-but-not-`valid`
/// budget or a degraded underlying scene marks them reduced; otherwise the fix is
/// valid. Deterministic and traceable.
pub(crate) fn classify(
    skew_ns: u64,
    extrapolation_ns: u64,
    rate_rps: f64,
    error: &AlignmentErrorBound,
    availability: SvsAvailability,
    policy: &ConformalPolicy,
) -> ConformalState {
    if skew_ns > policy.max_skew_ns() {
        return ConformalState::NonConformal(ConformalReason::ExcessiveSkew);
    }
    if extrapolation_ns > policy.max_extrapolation_ns() {
        return ConformalState::NonConformal(ConformalReason::ExcessiveExtrapolation);
    }
    // A non-finite rate fails closed alongside an over-limit one.
    if rate_rps > f64::from(policy.max_conformal_rate_rps()) || !rate_rps.is_finite() {
        return ConformalState::NonConformal(ConformalReason::ExcessiveRate);
    }
    if !error.within(policy.limited_error_rad()) {
        return ConformalState::NonConformal(ConformalReason::ErrorBudgetExceeded);
    }
    if !error.within(policy.valid_error_rad()) {
        return ConformalState::Limited(ConformalReason::ErrorBudgetExceeded);
    }
    if let SvsAvailability::Degraded(r) = availability {
        return ConformalState::Limited(ConformalReason::Availability(r));
    }
    ConformalState::Valid
}
