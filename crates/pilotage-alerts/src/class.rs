//! Static alert taxonomy: severity classes, their latching and declutter
//! behavior, the aural token each class owns, and the acknowledgement
//! lifecycle a managed alert occupies.
//!
//! Everything here is fixed content, the part AC 25.1322-1 separates from
//! live state: a class's severity, whether it latches, and which tone it
//! carries do not change at runtime. The live transition state lives in the
//! manager.

/// Severity class of an alert, ordered least to most urgent so [`Ord`]
/// yields the priority ranking directly ([`AlertClass::Warning`] is
/// greatest).
///
/// Warning outranks caution outranks advisory; status and maintenance sit
/// below and carry no aural.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlertClass {
    /// Maintenance information for ground crews; silent, and decluttered
    /// under unusual attitude.
    Maintenance,
    /// System status with no required flightcrew action; silent, and
    /// decluttered under unusual attitude.
    Status,
    /// Crew awareness, lowest aural tier (single chime).
    Advisory,
    /// Immediate crew awareness and likely subsequent action (triple
    /// chime).
    Caution,
    /// Immediate crew action; highest priority, continuous aural.
    Warning,
}

impl AlertClass {
    /// The static aural token this class owns. Status and maintenance are
    /// silent.
    pub const fn aural_token(self) -> AuralToken {
        match self {
            Self::Warning => AuralToken::ContinuousTone,
            Self::Caution => AuralToken::TripleChime,
            Self::Advisory => AuralToken::SingleChime,
            Self::Status | Self::Maintenance => AuralToken::Silent,
        }
    }

    /// Whether an alert of this class persists after its condition clears,
    /// until the crew acknowledges it. Warnings and cautions latch;
    /// advisories, status, and maintenance self-clear.
    pub const fn latches(self) -> bool {
        matches!(self, Self::Warning | Self::Caution)
    }

    /// Whether unusual-attitude declutter may hide this class. Warnings and
    /// cautions are always retained; everything below them declutters.
    pub const fn declutters_under_unusual(self) -> bool {
        matches!(self, Self::Advisory | Self::Status | Self::Maintenance)
    }
}

/// One-at-a-time aural command. The manager emits at most one token per
/// step; the audio backend (out of scope here) plays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuralToken {
    /// No tone this step.
    #[default]
    Silent,
    /// One chime: advisory onset.
    SingleChime,
    /// Three chimes: caution onset.
    TripleChime,
    /// Continuous tone that sounds every step until acknowledged: warning.
    ContinuousTone,
}

impl AuralToken {
    /// Whether the token sounds continuously — re-emitted every step while
    /// the alert stays active and unacknowledged — rather than as a
    /// one-shot at onset.
    pub const fn is_continuous(self) -> bool {
        matches!(self, Self::ContinuousTone)
    }
}

/// Where a managed alert sits in the acknowledgement lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertState {
    /// Condition present, not yet acknowledged: full visual and aural.
    Active,
    /// Condition present, crew acknowledged: visual persists, aural
    /// silenced.
    Acknowledged,
    /// Condition cleared but latched unacknowledged: visual persists,
    /// silent.
    LatchedCleared,
}

#[cfg(test)]
mod tests;
