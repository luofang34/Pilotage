//! Synthetic-vision availability: a finite, deterministic, traceable verdict.
//!
//! A missing, inconsistent, or untrusted input never yields a plausible normal
//! scene. [`SvsAvailability::assess`] maps the typed health of each input to a
//! verdict by a **fixed precedence**, so the same inputs always yield the same
//! verdict and the reason names the specific input that decided it. Health is
//! stated, never defaulted: an unknown input is [`InputHealth::Failed`], not
//! silently `Ok`.

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

    /// Decodes the wire byte, failing closed: an unknown value is
    /// [`Self::Failed`], never `Ok`.
    #[must_use]
    pub const fn from_u8_fail_closed(code: u8) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::Degraded,
            _ => Self::Failed,
        }
    }
}

/// The stated health of every synthetic-vision input.
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
