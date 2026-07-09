//! Per-signal validity: status, freshness policy, and combination rules.

/// The trust level of one displayed signal.
///
/// Ordered by severity: a combination of causes resolves to the worst.
/// Defaults to `Missing` — trust must be earned by data, not assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum SignalStatus {
    /// Fresh data from a source reporting itself good.
    Valid,
    /// Usable but reduced-confidence data (source quality degraded).
    Degraded,
    /// Data older than the staleness threshold; still shown, flagged.
    Stale,
    /// No data has ever been provided for this signal.
    #[default]
    Missing,
    /// The source declared the data invalid, or it is too old to show.
    Failed,
}

impl SignalStatus {
    /// Whether a value should be rendered at all (possibly flagged).
    pub fn shows_value(self) -> bool {
        matches!(self, Self::Valid | Self::Degraded | Self::Stale)
    }

    /// The worse of two statuses.
    pub fn worst(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }
}

/// A display value paired with the status a panel must honor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sig<T> {
    /// The value; meaningful only when `status.shows_value()`.
    pub value: T,
    /// How much to trust it.
    pub status: SignalStatus,
}

impl<T> Sig<T> {
    /// A valid signal.
    pub const fn valid(value: T) -> Self {
        Self {
            value,
            status: SignalStatus::Valid,
        }
    }

    /// A signal with an explicit status.
    pub const fn with_status(value: T, status: SignalStatus) -> Self {
        Self { value, status }
    }
}

impl Sig<f32> {
    /// A missing numeric signal (value is a quiet zero, never shown).
    pub const fn missing() -> Self {
        Self {
            value: 0.0,
            status: SignalStatus::Missing,
        }
    }
}

/// Freshness thresholds resolving an age into a status (ADR-0009's
/// staleness discipline applied to display data).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreshnessPolicy {
    /// Age at which data is flagged stale, in milliseconds.
    pub stale_after_ms: f32,
    /// Age at which data is no longer shown, in milliseconds.
    pub fail_after_ms: f32,
}

impl Default for FreshnessPolicy {
    fn default() -> Self {
        Self {
            stale_after_ms: 750.0,
            fail_after_ms: 3000.0,
        }
    }
}

impl FreshnessPolicy {
    /// Status from a group's age; `None` means never received.
    pub fn status_for_age(&self, age_ms: Option<f32>) -> SignalStatus {
        match age_ms {
            None => SignalStatus::Missing,
            Some(age) if age.is_nan() || age < 0.0 => SignalStatus::Missing,
            Some(age) if age >= self.fail_after_ms => SignalStatus::Failed,
            Some(age) if age >= self.stale_after_ms => SignalStatus::Stale,
            Some(_) => SignalStatus::Valid,
        }
    }
}

#[cfg(test)]
mod tests;
