//! The explicit availability profile: the freshness and accuracy limits an
//! intended function allocates, chosen by the receiver at the validation/decode
//! boundary rather than baked into the derivation.
//!
//! There is deliberately no default and no free function that picks a profile,
//! so SIM limits are never presented as operational limits by omission. The one
//! named profile shipped here is [`AvailabilityProfile::simulator`]; its checked
//! constructor refuses a zero or non-monotonic limit so a profile can never admit
//! a reading it should reject. The fields are private: a profile can only come
//! from the checked constructor or the controlled `simulator`, never a struct
//! literal that skips the check. The limits never travel on the wire — they are
//! receiver-controlled evaluation context, so the same frame can be judged under
//! different intended functions and the wire ABI is unchanged.

use crate::error::GeoError;

use super::InputHealth;

/// Identity of an availability profile — an intended-function allocation of the
/// freshness and accuracy limits. Compared for equality; a verdict carries it so
/// the limits it was judged against are traceable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvailabilityProfileId(pub u32);

/// The id of the simulator profile.
pub const SIMULATOR_PROFILE_ID: AvailabilityProfileId = AvailabilityProfileId(1);

/// The freshness and accuracy limits an intended function allocates, selected at
/// the validation/decode boundary. There is deliberately **no** `Default` and no
/// free function that picks one: a caller must choose a profile explicitly, so
/// SIM limits are never presented as operational limits by omission.
///
/// The fields are private and the only ways to obtain a value are the checked
/// [`AvailabilityProfile::new`] and the controlled [`AvailabilityProfile::simulator`].
/// Within each pair the *fresh* limit must be strictly tighter than the *usable*
/// limit and both must be non-zero, and `new` enforces that — so a struct literal
/// that skips the check does not compile:
///
/// ```compile_fail
/// use pilotage_geo::{AvailabilityProfile, AvailabilityProfileId};
/// // Non-monotonic (fresh age 999 > usable age 1) AND unconstructable: the
/// // fields are private, so this cannot bypass `new`'s monotonicity check.
/// let _ = AvailabilityProfile {
///     id: AvailabilityProfileId(9),
///     version: 1,
///     fresh_age_ns: 999,
///     usable_age_ns: 1,
///     fresh_pos_mm: 1,
///     usable_pos_mm: 2,
///     fresh_att_mrad: 1,
///     usable_att_mrad: 2,
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvailabilityProfile {
    id: AvailabilityProfileId,
    version: u16,
    fresh_age_ns: u64,
    usable_age_ns: u64,
    fresh_pos_mm: u32,
    usable_pos_mm: u32,
    fresh_att_mrad: u32,
    usable_att_mrad: u32,
}

impl AvailabilityProfile {
    /// The published simulator profile: the SIM freshness and accuracy
    /// allocation. This is a placeholder for an intended-function allocation, not
    /// an operational limit.
    #[must_use]
    pub const fn simulator() -> Self {
        Self {
            id: SIMULATOR_PROFILE_ID,
            version: 1,
            // A conformal scene at typical closing speeds is visibly wrong within
            // a few hundred milliseconds of latency; beyond ~1 s the registration
            // error is unbounded for any useful motion.
            fresh_age_ns: 200_000_000,
            usable_age_ns: 1_000_000_000,
            // A few meters of registration error is visible but still orienting;
            // tens of meters place symbology on the wrong feature.
            fresh_pos_mm: 5_000,
            usable_pos_mm: 50_000,
            // About half a degree is visible at range but orienting; several
            // degrees swing the horizon off the true one.
            fresh_att_mrad: 10,
            usable_att_mrad: 50,
        }
    }

    /// Builds a profile, failing closed on a zero or non-monotonic limit: within
    /// each pair the fresh limit must be strictly less than the usable limit and
    /// both must be non-zero, so the profile cannot admit a reading it should
    /// reject.
    ///
    /// # Errors
    ///
    /// [`GeoError::InvalidAvailabilityProfile`] naming the offending pair.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AvailabilityProfileId,
        version: u16,
        fresh_age_ns: u64,
        usable_age_ns: u64,
        fresh_pos_mm: u32,
        usable_pos_mm: u32,
        fresh_att_mrad: u32,
        usable_att_mrad: u32,
    ) -> Result<Self, GeoError> {
        let monotonic_u64 = |fresh: u64, usable: u64| fresh > 0 && usable > fresh;
        let monotonic_u32 = |fresh: u32, usable: u32| fresh > 0 && usable > fresh;
        if !monotonic_u64(fresh_age_ns, usable_age_ns) {
            return Err(GeoError::InvalidAvailabilityProfile { field: "age" });
        }
        if !monotonic_u32(fresh_pos_mm, usable_pos_mm) {
            return Err(GeoError::InvalidAvailabilityProfile { field: "position" });
        }
        if !monotonic_u32(fresh_att_mrad, usable_att_mrad) {
            return Err(GeoError::InvalidAvailabilityProfile { field: "attitude" });
        }
        Ok(Self {
            id,
            version,
            fresh_age_ns,
            usable_age_ns,
            fresh_pos_mm,
            usable_pos_mm,
            fresh_att_mrad,
            usable_att_mrad,
        })
    }

    /// Profile identity.
    #[must_use]
    pub const fn id(&self) -> AvailabilityProfileId {
        self.id
    }
    /// Profile content version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    /// Age at/under which a reading is fully fresh, nanoseconds.
    #[must_use]
    pub const fn fresh_age_ns(&self) -> u64 {
        self.fresh_age_ns
    }
    /// Age beyond which a reading is unusable, nanoseconds.
    #[must_use]
    pub const fn usable_age_ns(&self) -> u64 {
        self.usable_age_ns
    }
    /// Position 1-sigma accuracy (per axis) at/under which it is fresh, mm.
    #[must_use]
    pub const fn fresh_pos_mm(&self) -> u32 {
        self.fresh_pos_mm
    }
    /// Position 1-sigma accuracy beyond which it is unusable, mm.
    #[must_use]
    pub const fn usable_pos_mm(&self) -> u32 {
        self.usable_pos_mm
    }
    /// Attitude 1-sigma accuracy at/under which it is fresh, milliradians.
    #[must_use]
    pub const fn fresh_att_mrad(&self) -> u32 {
        self.fresh_att_mrad
    }
    /// Attitude 1-sigma accuracy beyond which it is unusable, milliradians.
    #[must_use]
    pub const fn usable_att_mrad(&self) -> u32 {
        self.usable_att_mrad
    }

    /// The health a position accuracy (mm) contributes under this profile.
    #[must_use]
    pub(super) const fn position_mm_health(&self, mm: u32) -> InputHealth {
        if mm > self.usable_pos_mm {
            InputHealth::Failed
        } else if mm > self.fresh_pos_mm {
            InputHealth::Degraded
        } else {
            InputHealth::Ok
        }
    }

    /// The health an attitude accuracy (milliradians) contributes.
    #[must_use]
    pub(super) const fn attitude_mrad_health(&self, mrad: u32) -> InputHealth {
        if mrad > self.usable_att_mrad {
            InputHealth::Failed
        } else if mrad > self.fresh_att_mrad {
            InputHealth::Degraded
        } else {
            InputHealth::Ok
        }
    }

    /// The health an age (nanoseconds) contributes.
    #[must_use]
    pub(super) const fn age_health(&self, age_ns: u64) -> InputHealth {
        if age_ns > self.usable_age_ns {
            InputHealth::Failed
        } else if age_ns > self.fresh_age_ns {
            InputHealth::Degraded
        } else {
            InputHealth::Ok
        }
    }
}

#[cfg(test)]
mod tests;
