//! What the onboard navigator says of one of its position sensors beside
//! its solution: whether the sensor agrees with the rest, and whether the
//! navigator has excluded it.
//!
//! The navigator judges its sensors; the engine neither repeats nor
//! second-guesses the judgment. It keeps the statement with the solution
//! it came with and reports it, so a display can say that a sensor was
//! excluded instead of showing a position source that silently changed.

use navigate_contract::{MonotonicNanos, NavigationSolution, SensorClass};

/// The navigator's judgment of one position sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceIntegrityState {
    /// The sensor agrees with the navigator's other sources.
    Consistent,
    /// The sensor disagrees with the other sources beyond the navigator's
    /// caution bound and still contributes.
    Disagreeing,
    /// The navigator has excluded the sensor: the solution goes on
    /// without it.
    Excluded,
}

/// The navigator's statement of one position sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceIntegrity {
    /// The sensor's class.
    pub class: SensorClass,
    /// The navigator's judgment of it.
    pub state: SourceIntegrityState,
    /// Since when the judgment has held, on the navigator's own clock;
    /// `None` when the navigator does not say.
    pub since: Option<MonotonicNanos>,
}

/// One solution of the onboard navigator as its host offers it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnboardSolution {
    /// The solution.
    pub solution: NavigationSolution,
    /// The navigator's statement of a position sensor it monitors; `None`
    /// when it monitors none or says nothing.
    pub source_integrity: Option<SourceIntegrity>,
}

impl OnboardSolution {
    /// `solution` with the navigator's statement of a position sensor.
    #[must_use]
    pub const fn with_source_integrity(
        solution: NavigationSolution,
        source_integrity: Option<SourceIntegrity>,
    ) -> Self {
        Self {
            solution,
            source_integrity,
        }
    }
}

/// A solution offered alone comes with no statement.
impl From<&NavigationSolution> for OnboardSolution {
    fn from(solution: &NavigationSolution) -> Self {
        Self {
            solution: *solution,
            source_integrity: None,
        }
    }
}
