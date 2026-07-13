//! The intended-function policy that bounds conformal projection: the
//! extrapolation, skew, angular-rate, and total-error allocations a conformal
//! HUD function is willing to draw registered symbology under.
//!
//! This is the conformal sibling of [`crate::AvailabilityProfile`]: availability
//! allocates the freshness and accuracy limits that decide whether a synthetic
//! scene may be drawn at all; this allocates the *registration* limits that
//! decide whether that scene may be drawn **conformally** (locked to the outside
//! world through the camera). The two are layered, not duplicated — a conformal
//! verdict first consumes the [`crate::SvsAvailability`] verdict and only then
//! applies these registration limits.
//!
//! There is deliberately no `Default` and no free function that picks a policy,
//! so SIM limits are never presented as operational limits by omission. The one
//! named policy shipped here is [`ConformalPolicy::simulator`]; its numbers are
//! SIM benchmark data implying no aircraft or display approval. The checked
//! [`ConformalPolicy::new`] refuses a zero, non-finite, or non-monotonic limit so
//! a policy can never admit a registration it should reject, and the fields are
//! private so a struct literal cannot skip the check.

/// Why a conformal input could not be validated. A limit or a resolved geometry
/// value that is zero, non-finite, non-monotonic, or otherwise malformed is
/// refused, never clamped into range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConformalError {
    /// A policy limit was zero, non-finite, or its `valid` bound was not strictly
    /// tighter than its `limited` bound (a non-monotonic error allocation could
    /// admit a registration it should mark or drop).
    #[error("conformal policy limit {field} is invalid (zero, non-finite, or non-monotonic)")]
    InvalidPolicy {
        /// The offending limit.
        field: &'static str,
    },
    /// A resolved [`crate::ViewGeometry`] field was malformed: an incomplete
    /// calibration reference, a non-unit body→camera rotation, a non-positive or
    /// non-finite field of view, or a negative/non-finite alignment bound.
    #[error("conformal view geometry field {field} is invalid")]
    InvalidGeometry {
        /// The offending geometry field.
        field: &'static str,
    },
}

/// Identity of a conformal policy — an intended-function allocation of the
/// registration limits. Compared for equality; a verdict carries it so the
/// limits it was judged against are traceable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConformalPolicyId(pub u32);

/// The id of the simulator conformal policy.
pub const SIMULATOR_CONFORMAL_POLICY_ID: ConformalPolicyId = ConformalPolicyId(1);

/// The registration limits an intended conformal function allocates, selected at
/// the evaluation boundary. There is deliberately **no** `Default`: a caller must
/// choose a policy explicitly, so SIM limits are never presented as operational
/// limits by omission.
///
/// The fields are private and the only ways to obtain a value are the checked
/// [`ConformalPolicy::new`] and the controlled [`ConformalPolicy::simulator`]. A
/// struct literal that skips the monotonicity check does not compile:
///
/// ```compile_fail
/// use pilotage_geo::{ConformalPolicy, ConformalPolicyId};
/// let _ = ConformalPolicy {
///     id: ConformalPolicyId(9),
///     version: 1,
///     max_extrapolation_ns: 1,
///     max_skew_ns: 1,
///     max_conformal_rate_rps: 1.0,
///     valid_error_rad: 9.0,
///     limited_error_rad: 1.0,
///     reference_range_m: 50.0,
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConformalPolicy {
    id: ConformalPolicyId,
    version: u16,
    max_extrapolation_ns: u64,
    max_skew_ns: u64,
    max_conformal_rate_rps: f32,
    valid_error_rad: f64,
    limited_error_rad: f64,
    reference_range_m: f64,
}

impl ConformalPolicy {
    /// The published simulator policy. Its numbers are SIM benchmark data — a
    /// placeholder intended-function allocation, not an operational or display
    /// approval:
    ///
    /// - beyond ~50 ms of extrapolation past the newest snapshot a maneuvering
    ///   aircraft's registration is no longer trustworthy;
    /// - attitude and position must be co-timed to within ~10 ms to share one
    ///   conformal fix;
    /// - above ~1 rad/s (~57°/s) body rate the smear over any residual timing
    ///   error swamps the budget, so conformal cues are suppressed;
    /// - sub-degree (~0.57°) total registration error is drawn as fully
    ///   registered; up to ~1.7° is drawn but marked reduced; beyond that the
    ///   cues are removed;
    /// - 50 m is the representative near-field conformal-feature range at which a
    ///   positional error is converted to an angular parallax bound (the same
    ///   reference range the calibration design-eye allowance uses).
    #[must_use]
    pub const fn simulator() -> Self {
        Self {
            id: SIMULATOR_CONFORMAL_POLICY_ID,
            version: 1,
            max_extrapolation_ns: 50_000_000,
            max_skew_ns: 10_000_000,
            max_conformal_rate_rps: 1.0,
            valid_error_rad: 0.010,
            limited_error_rad: 0.030,
            reference_range_m: 50.0,
        }
    }

    /// Builds a policy, failing closed on a zero, non-finite, or non-monotonic
    /// limit: every duration and angle must be strictly positive and finite, and
    /// the `valid` error bound must be strictly tighter than the `limited` bound,
    /// so the policy cannot admit a registration it should mark or drop.
    ///
    /// # Errors
    ///
    /// [`ConformalError::InvalidPolicy`] naming the offending limit.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ConformalPolicyId,
        version: u16,
        max_extrapolation_ns: u64,
        max_skew_ns: u64,
        max_conformal_rate_rps: f32,
        valid_error_rad: f64,
        limited_error_rad: f64,
        reference_range_m: f64,
    ) -> Result<Self, ConformalError> {
        let bad = |field| Err(ConformalError::InvalidPolicy { field });
        if max_extrapolation_ns == 0 {
            return bad("max_extrapolation_ns");
        }
        if max_skew_ns == 0 {
            return bad("max_skew_ns");
        }
        if !(max_conformal_rate_rps.is_finite() && max_conformal_rate_rps > 0.0) {
            return bad("max_conformal_rate_rps");
        }
        if !(reference_range_m.is_finite() && reference_range_m > 0.0) {
            return bad("reference_range_m");
        }
        let monotonic = valid_error_rad.is_finite()
            && limited_error_rad.is_finite()
            && valid_error_rad > 0.0
            && limited_error_rad > valid_error_rad;
        if !monotonic {
            return bad("error_bound");
        }
        Ok(Self {
            id,
            version,
            max_extrapolation_ns,
            max_skew_ns,
            max_conformal_rate_rps,
            valid_error_rad,
            limited_error_rad,
            reference_range_m,
        })
    }

    /// Policy identity.
    #[must_use]
    pub const fn id(&self) -> ConformalPolicyId {
        self.id
    }
    /// Policy content version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    /// Extrapolation horizon (past the newest snapshot, or before the oldest)
    /// beyond which conformal registration is refused, nanoseconds.
    #[must_use]
    pub const fn max_extrapolation_ns(&self) -> u64 {
        self.max_extrapolation_ns
    }
    /// Maximum attitude/position co-timing skew for one conformal fix,
    /// nanoseconds.
    #[must_use]
    pub const fn max_skew_ns(&self) -> u64 {
        self.max_skew_ns
    }
    /// Body angular rate above which conformal cues are suppressed, rad/s.
    #[must_use]
    pub const fn max_conformal_rate_rps(&self) -> f32 {
        self.max_conformal_rate_rps
    }
    /// Total alignment error at/under which a fix is fully registered
    /// ([`crate::ConformalState::Valid`]), radians.
    #[must_use]
    pub const fn valid_error_rad(&self) -> f64 {
        self.valid_error_rad
    }
    /// Total alignment error at/under which a fix is drawn but marked reduced
    /// ([`crate::ConformalState::Limited`]); beyond it the cues are removed,
    /// radians.
    #[must_use]
    pub const fn limited_error_rad(&self) -> f64 {
        self.limited_error_rad
    }
    /// Reference range at which a positional error is converted to an angular
    /// parallax bound, meters.
    #[must_use]
    pub const fn reference_range_m(&self) -> f64 {
        self.reference_range_m
    }
}
