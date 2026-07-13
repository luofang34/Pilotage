//! Synthetic-vision availability: a finite, deterministic, traceable verdict
//! **derived** from validated inputs, never self-reported by a wire producer.
//!
//! [`SvsAvailability::assess`] maps the typed health of each input to a verdict
//! by a **fixed precedence**, so the same inputs always yield the same verdict
//! and the reason names the specific input that decided it. The health of the
//! inputs the contract can check for itself — position, attitude, and
//! time/coherence — is *derived* ([`derive_inputs`]) from the actual position
//! and attitude stamps and the frame reference time; only the inputs the
//! contract cannot check (navigation-integrity monitor, calibration, database,
//! coverage, renderer) are producer-stated ([`ExternalHealth`]). An untrusted
//! reading can never produce an [`SvsAvailability::Available`] scene.

use pilotage_frames::Epoch;

use crate::identity::{IntegrityLevel, PositionQuality, StatedAttitude, StatedPosition};

mod profile;

pub use profile::{AvailabilityProfile, AvailabilityProfileId, SIMULATOR_PROFILE_ID};

/// The finite set of reasons synthetic vision can be degraded or unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AvailabilityReason {
    /// No fault; the scene is fully available.
    Nominal = 0,
    /// Position source missing, out of range, or untrusted.
    Position = 1,
    /// Attitude source missing or untrusted.
    Attitude = 2,
    /// Navigation integrity insufficient.
    Integrity = 3,
    /// Time base or cross-source coherence broken.
    TimeCoherence = 4,
    /// Camera/eye calibration missing or invalid.
    Calibration = 5,
    /// Terrain/obstacle database missing or stale.
    Database = 6,
    /// The database does not cover the current position.
    Coverage = 7,
    /// The renderer cannot produce a trustworthy frame.
    Renderer = 8,
}

impl AvailabilityReason {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes the wire byte, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Nominal),
            1 => Some(Self::Position),
            2 => Some(Self::Attitude),
            3 => Some(Self::Integrity),
            4 => Some(Self::TimeCoherence),
            5 => Some(Self::Calibration),
            6 => Some(Self::Database),
            7 => Some(Self::Coverage),
            8 => Some(Self::Renderer),
            _ => None,
        }
    }
}

/// The stated health of one synthetic-vision input. There is no default: a
/// caller that does not know an input's health states [`Self::Failed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InputHealth {
    /// Contributing normally.
    Ok = 0,
    /// Usable but reduced; contributes a degraded scene.
    Degraded = 1,
    /// Missing, inconsistent, or untrusted; contributes no scene.
    Failed = 2,
}

impl InputHealth {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes a wire byte strictly: an unknown value is `None`, so the ABI can
    /// refuse non-canonical data rather than silently coercing it (which would
    /// also make decode-then-encode change the bytes).
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Ok),
            1 => Some(Self::Degraded),
            2 => Some(Self::Failed),
            _ => None,
        }
    }

    /// Decodes a wire byte defensively: an unknown value is [`Self::Failed`],
    /// never `Ok`. Used where a non-strict, fail-safe interpretation is wanted;
    /// the ABI itself decodes strictly via [`Self::from_u8`].
    #[must_use]
    pub const fn from_u8_fail_closed(code: u8) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::Degraded,
            _ => Self::Failed,
        }
    }
}

/// The producer-stated health of the inputs this contract cannot verify for
/// itself: the navigation-integrity monitor and the calibration, database,
/// coverage, and renderer subsystems. Position, attitude, and time/coherence
/// health are never stated here — they are derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalHealth {
    /// Navigation-integrity monitor health (e.g. RAIM/SBAS protection level
    /// versus alert limit) — an external monitor the contract cannot recompute.
    pub integrity: InputHealth,
    /// Camera/eye calibration health.
    pub calibration: InputHealth,
    /// Terrain/obstacle database health.
    pub database: InputHealth,
    /// Database coverage health at the current position.
    pub coverage: InputHealth,
    /// Renderer health.
    pub renderer: InputHealth,
}

impl ExternalHealth {
    /// All external inputs failed — the fail-closed default when nothing is
    /// stated.
    #[must_use]
    pub const fn all_failed() -> Self {
        let f = InputHealth::Failed;
        Self {
            integrity: f,
            calibration: f,
            database: f,
            coverage: f,
            renderer: f,
        }
    }
}

/// The health of every synthetic-vision input, position/attitude/time-coherence
/// derived and the rest producer-stated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvsInputs {
    /// Position source health.
    pub position: InputHealth,
    /// Attitude source health.
    pub attitude: InputHealth,
    /// Navigation integrity health.
    pub integrity: InputHealth,
    /// Time base / cross-source coherence health.
    pub time_coherence: InputHealth,
    /// Camera/eye calibration health.
    pub calibration: InputHealth,
    /// Terrain/obstacle database health.
    pub database: InputHealth,
    /// Database coverage health at the current position.
    pub coverage: InputHealth,
    /// Renderer health.
    pub renderer: InputHealth,
}

impl SvsInputs {
    /// All inputs failed — the fail-closed default when nothing is known.
    #[must_use]
    pub const fn all_failed() -> Self {
        let f = InputHealth::Failed;
        Self {
            position: f,
            attitude: f,
            integrity: f,
            time_coherence: f,
            calibration: f,
            database: f,
            coverage: f,
            renderer: f,
        }
    }

    /// The inputs in fixed precedence order (safety-load-bearing first), paired
    /// with the reason each maps to. The order is the contract: it decides which
    /// reason wins when several inputs fault.
    const fn in_precedence(&self) -> [(AvailabilityReason, InputHealth); 8] {
        [
            (AvailabilityReason::Position, self.position),
            (AvailabilityReason::Attitude, self.attitude),
            (AvailabilityReason::Integrity, self.integrity),
            (AvailabilityReason::TimeCoherence, self.time_coherence),
            (AvailabilityReason::Calibration, self.calibration),
            (AvailabilityReason::Database, self.database),
            (AvailabilityReason::Coverage, self.coverage),
            (AvailabilityReason::Renderer, self.renderer),
        ]
    }
}

/// Maps an integrity level to the health a reading at that level contributes:
/// only `Trusted` is fully usable, `Monitored` degrades, and an untrusted or
/// unmonitored reading fails closed.
#[must_use]
pub const fn health_from_integrity(level: IntegrityLevel) -> InputHealth {
    match level {
        IntegrityLevel::Trusted => InputHealth::Ok,
        IntegrityLevel::Monitored => InputHealth::Degraded,
        IntegrityLevel::Untrusted | IntegrityLevel::Unknown => InputHealth::Failed,
    }
}

/// The worse of two healths (`Ok` < `Degraded` < `Failed`), so a reading is no
/// healthier than its weakest attribute.
#[must_use]
const fn worse(a: InputHealth, b: InputHealth) -> InputHealth {
    if a.to_u8() >= b.to_u8() { a } else { b }
}

/// Maps a position accuracy to the health it contributes under `profile`: the
/// worse of the horizontal and vertical axes decides.
#[must_use]
const fn health_from_position_quality(
    q: PositionQuality,
    profile: &AvailabilityProfile,
) -> InputHealth {
    worse(
        profile.position_mm_health(q.horizontal_mm),
        profile.position_mm_health(q.vertical_mm),
    )
}

/// Derives time/coherence health from the position and attitude stamps and the
/// frame reference time under `profile`, failing closed: incompatible
/// clocks/scales, a future sample, no declared coherent snapshot, or an unusable
/// age all fail; a merely stale but usable pair degrades.
#[must_use]
fn derive_time_coherence(
    position: &StatedPosition,
    attitude: &StatedAttitude,
    reference_time: Epoch,
    profile: &AvailabilityProfile,
) -> InputHealth {
    // A coherent scene requires the position and attitude to be one declared
    // coherent snapshot on one time base.
    if !position.stamp.coherent_with(&attitude.stamp) {
        return InputHealth::Failed;
    }
    // Both ages must be physical durations relative to the reference time; a
    // future sample or an incompatible clock/scale fails closed.
    let (Ok(pos_age), Ok(att_age)) = (
        position.stamp.age_ns(reference_time),
        attitude.stamp.age_ns(reference_time),
    ) else {
        return InputHealth::Failed;
    };
    profile.age_health(pos_age.max(att_age))
}

/// Derives the full input health from the validated position and attitude, the
/// producer-stated external health, the frame reference time, and the selected
/// availability `profile`. Position, attitude, and time/coherence are computed
/// here against the profile's limits; the rest are taken as stated.
#[must_use]
pub fn derive_inputs(
    position: &StatedPosition,
    attitude: &StatedAttitude,
    external: &ExternalHealth,
    reference_time: Epoch,
    profile: &AvailabilityProfile,
) -> SvsInputs {
    SvsInputs {
        position: worse(
            health_from_integrity(position.stamp.integrity),
            health_from_position_quality(position.quality, profile),
        ),
        attitude: worse(
            health_from_integrity(attitude.stamp.integrity),
            profile.attitude_mrad_health(attitude.quality.angular_mrad),
        ),
        integrity: external.integrity,
        time_coherence: derive_time_coherence(position, attitude, reference_time, profile),
        calibration: external.calibration,
        database: external.database,
        coverage: external.coverage,
        renderer: external.renderer,
    }
}

/// The synthetic-vision availability verdict. A degraded or unavailable verdict
/// always carries the specific [`AvailabilityReason`] that decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvsAvailability {
    /// Fully available.
    Available,
    /// Reduced but usable, for the given reason.
    Degraded(AvailabilityReason),
    /// Not usable, for the given reason.
    Unavailable(AvailabilityReason),
}

impl SvsAvailability {
    /// Assesses availability from the inputs by fixed precedence: the first
    /// failed input (in order) makes the scene unavailable for its reason; else
    /// the first degraded input degrades it; else it is available. Deterministic
    /// and traceable — the reason names the deciding input.
    #[must_use]
    pub fn assess(inputs: &SvsInputs) -> Self {
        let ordered = inputs.in_precedence();
        for (reason, health) in ordered {
            if health == InputHealth::Failed {
                return Self::Unavailable(reason);
            }
        }
        for (reason, health) in ordered {
            if health == InputHealth::Degraded {
                return Self::Degraded(reason);
            }
        }
        Self::Available
    }

    /// The reason behind this verdict ([`AvailabilityReason::Nominal`] when
    /// available).
    #[must_use]
    pub const fn reason(self) -> AvailabilityReason {
        match self {
            Self::Available => AvailabilityReason::Nominal,
            Self::Degraded(r) | Self::Unavailable(r) => r,
        }
    }

    /// Whether a normal scene may be drawn (available only).
    #[must_use]
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

#[cfg(test)]
mod tests;
