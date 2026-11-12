//! What the plan is steered on, for a display: the flight phase that sets
//! the scale of its deviations, and the position source flown with the
//! engine's judgment of it.
//!
//! This is display context (ADR-0031): never an input to control
//! validation. It is stated so a display names its source and scales its
//! deviations from what the engine says, never from what it assumes.

use navigate_contract::{SensorClass, SourceComposition};

use crate::config::OwnshipSource;

use super::{MissionEngine, NavQuality, StartWithheldReason};

#[cfg(test)]
mod tests;

/// The phase of flight that sets the required navigation performance, and
/// with it the scale a display gives its lateral deviation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FlightPhase {
    /// Between the terminal areas: every leg with no terminal or approach
    /// role.
    Enroute,
    /// A departure, an arrival, or the initial and intermediate segments
    /// of an approach.
    Terminal,
    /// The final approach segment.
    Approach,
    /// The missed approach.
    Missed,
}

/// Where the position the plan is steered on comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PositionSource {
    /// A simulator's truth oracle, fused as a synthesized fix. A display
    /// must not name it as a navigation sensor.
    SimulationTruth,
    /// The flight controller's own estimate.
    VehicleEstimate,
    /// The onboard navigator's solution.
    OnboardNavigation,
}

/// The engine's judgment of the position it steers on, against the limits
/// the mission flies under. The navigator's own grade travels inside as
/// `quality`: a solution the navigator grades unusable can still be the
/// one flown, by dead reckoning.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum SteeringIntegrity {
    /// A solution within the limits is flown.
    WithinLimits {
        /// The navigator's grade of the solution.
        quality: NavQuality,
        /// Its 1-sigma horizontal uncertainty, metres.
        horizontal_1sigma_m: f64,
    },
    /// A solution beyond the limits is flown all the same: the plan goes
    /// on by dead reckoning.
    DeadReckoning {
        /// The navigator's grade of the solution.
        quality: NavQuality,
        /// Its 1-sigma horizontal uncertainty, metres.
        horizontal_1sigma_m: f64,
    },
    /// No position is steered on. Whatever the vehicle is commanded, it is
    /// not a position-based command.
    NoSolution {
        /// Why the source gives none.
        reason: StartWithheldReason,
    },
}

/// The position source the plan is steered on, and the engine's judgment
/// of it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct SteeringSource {
    /// Where the position comes from.
    pub source: PositionSource,
    /// The sensor classes that contribute to the solution flown. `None`
    /// while none is flown, and under [`PositionSource::SimulationTruth`],
    /// whose position comes from no sensor. A display names a sensor only
    /// when its class is among them.
    pub contributing: Option<SourceComposition>,
    /// The engine's judgment of the position.
    pub integrity: SteeringIntegrity,
}

/// The kind of position the plan is steered on, as an instrument
/// annunciates it beside its guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PositionKind {
    /// Satellite navigation is the only position aid.
    Satellite,
    /// A camera is the only position aid.
    Visual,
    /// No position aid contributes: the solution is carried by inertial
    /// and air data alone, within its limits.
    Inertial,
    /// A solution beyond its limits is flown by dead reckoning.
    DeadReckoning,
    /// Several position aids contribute, or the position is an estimate
    /// whose aids are not stated: the flight controller's own, another
    /// navigator's, or a simulator's truth.
    Blended,
}

/// The position aids of `contributing`, as the kinds an instrument names.
fn kind_of(contributing: SourceComposition) -> PositionKind {
    let satellite = contributing.contains(SensorClass::Gnss);
    let visual = contributing.contains(SensorClass::VisualLandmark)
        || contributing.contains(SensorClass::VisualOdometry);
    // Aids an instrument has no name for, and estimates that do not state
    // what aided them.
    let other = [
        SensorClass::Celestial,
        SensorClass::RadioNavigation,
        SensorClass::MagneticAnomaly,
        SensorClass::FcState,
        SensorClass::NavigationSolution,
    ]
    .into_iter()
    .any(|class| contributing.contains(class));
    match (satellite, visual, other) {
        (false, false, false) => PositionKind::Inertial,
        (true, false, false) => PositionKind::Satellite,
        (false, true, false) => PositionKind::Visual,
        _ => PositionKind::Blended,
    }
}

impl SteeringSource {
    /// The kind of position steered on; `None` while none is. Dead
    /// reckoning is named whatever contributes. A simulator's truth names
    /// no sensor, so it is a blend.
    #[must_use]
    pub fn position_kind(&self) -> Option<PositionKind> {
        match self.integrity {
            SteeringIntegrity::NoSolution { .. } => None,
            SteeringIntegrity::DeadReckoning { .. } => Some(PositionKind::DeadReckoning),
            SteeringIntegrity::WithinLimits { .. } => {
                Some(self.contributing.map_or(PositionKind::Blended, kind_of))
            }
        }
    }

    /// A steering source from its parts.
    #[must_use]
    pub const fn new(
        source: PositionSource,
        contributing: Option<SourceComposition>,
        integrity: SteeringIntegrity,
    ) -> Self {
        Self {
            source,
            contributing,
            integrity,
        }
    }
}

impl MissionEngine {
    /// The flight phase of the active leg. It follows the roles of the
    /// plan's legs, and a plan whose legs carry no terminal or approach
    /// role is en route throughout. The plans this engine builds carry no
    /// such role.
    #[must_use]
    pub const fn flight_phase(&self) -> FlightPhase {
        FlightPhase::Enroute
    }

    /// The position source steered on and its integrity, from the state
    /// the latest [`MissionEngine::tick`] that read the source left.
    #[must_use]
    pub fn steering_source(&self) -> SteeringSource {
        let source = match self.config.ownship_source {
            OwnshipSource::SimulationTruth => PositionSource::SimulationTruth,
            OwnshipSource::VehicleEstimate { .. } => PositionSource::VehicleEstimate,
            OwnshipSource::OnboardNavigation { .. } => PositionSource::OnboardNavigation,
        };
        let Some(solution) = self.last_solution.as_ref() else {
            return SteeringSource {
                source,
                contributing: None,
                integrity: SteeringIntegrity::NoSolution {
                    reason: self
                        .no_solution
                        .unwrap_or(StartWithheldReason::NavigationNotStarted),
                },
            };
        };
        let quality = solution.integrity.quality.into();
        let horizontal_1sigma_m = solution.integrity.horizontal_1sigma_m;
        let integrity = if self.onboard.dead_reckoned.is_some() {
            SteeringIntegrity::DeadReckoning {
                quality,
                horizontal_1sigma_m,
            }
        } else {
            SteeringIntegrity::WithinLimits {
                quality,
                horizontal_1sigma_m,
            }
        };
        SteeringSource {
            source,
            // The truth oracle is fused as a synthesized fix whose class
            // is not a sensor the vehicle carries.
            contributing: (source != PositionSource::SimulationTruth)
                .then_some(solution.integrity.contributing),
            integrity,
        }
    }
}
