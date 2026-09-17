//! The scenario document: waypoints, scripted operator messages, and the
//! outcome the verifier holds the flight to.
//!
//! The expected outcome lives here, written by the scenario author. The
//! verifier reads it from this document and never from the classifier, so
//! a wrong parse shows as a failed run and cannot grade itself.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::PilotError;

/// The name every scenario gives the launch point.
pub(crate) const HOME: &str = "HOME";

/// What the operator wants the vehicle to do at the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum Arrival {
    /// Descend, touch down and disarm.
    Land,
    /// Stay at the destination at cruise height.
    Hold,
}

/// A horizontal position in metres north and east of the launch point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Waypoint {
    /// Metres north of the launch point.
    pub north_m: f64,
    /// Metres east of the launch point.
    pub east_m: f64,
}

/// A destination and an arrival behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Intent {
    /// Waypoint name, or `HOME`.
    pub target: String,
    /// Behaviour at the destination.
    pub on_arrival: Arrival,
}

/// The condition that releases a scripted operator message.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "when", rename_all = "snake_case")]
pub(crate) enum Trigger {
    /// Released as soon as control is held.
    Start,
    /// Released after the vehicle is en route for this many seconds
    /// without a break.
    EnrouteFor {
        /// Continuous en-route time in seconds.
        seconds: f64,
    },
}

/// One scripted operator message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct OperatorMessage {
    /// The text given to the classifier, word for word.
    pub text: String,
    /// The release condition.
    pub trigger: Trigger,
    /// The intent the author means. It grades the parse in the run
    /// record and has no effect on the flight.
    pub means: Intent,
}

/// A checkpoint: truth must come this close to a waypoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Approach {
    /// The waypoint to come close to.
    pub target: String,
    /// The horizontal distance that satisfies the checkpoint, in metres.
    pub within_m: f64,
}

/// The outcome the verifier holds the flight to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Expectation {
    /// Where the flight must end.
    pub final_target: String,
    /// How the flight must end.
    pub final_on_arrival: Arrival,
    /// Checkpoints truth must satisfy, in this order, before the end
    /// state counts. Without them a vehicle that goes up and comes down
    /// at the launch point would pass a flight that ends at `HOME`.
    #[serde(default)]
    pub must_approach: Vec<Approach>,
    /// Waypoints the vehicle must stay away from, by the arrival radius.
    #[serde(default)]
    pub must_not_reach: Vec<String>,
    /// Seconds from the first message to the verdict deadline.
    pub timeout_s: f64,
}

/// A complete scenario document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Scenario {
    /// Stable identifier, copied into the run record.
    pub id: String,
    /// Named waypoints. `HOME` is implicit at the origin.
    pub waypoints: BTreeMap<String, Waypoint>,
    /// Height above the launch point for the cruise, in metres.
    pub cruise_height_m: f64,
    /// Horizontal distance that counts as arrival, in metres.
    pub arrival_radius_m: f64,
    /// Scripted operator messages, in release order.
    pub messages: Vec<OperatorMessage>,
    /// The outcome the verifier holds the flight to.
    pub expect: Expectation,
}

impl Scenario {
    /// Reads and validates a scenario document.
    pub(crate) async fn load(path: &Path) -> Result<Self, PilotError> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|source| PilotError::ScenarioRead {
                path: path.to_owned(),
                source,
            })?;
        let scenario: Self =
            serde_json::from_slice(&bytes).map_err(|source| PilotError::ScenarioParse {
                path: path.to_owned(),
                source,
            })?;
        scenario.validate()?;
        Ok(scenario)
    }

    /// The position of a named waypoint. `HOME` is the origin.
    pub(crate) fn position(&self, name: &str) -> Option<Waypoint> {
        if name == HOME {
            return Some(Waypoint {
                north_m: 0.0,
                east_m: 0.0,
            });
        }
        self.waypoints.get(name).copied()
    }

    /// Every name the classifier may answer with, `HOME` last.
    pub(crate) fn target_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.waypoints.keys().cloned().collect();
        names.push(HOME.to_owned());
        names
    }

    fn validate(&self) -> Result<(), PilotError> {
        let invalid = |detail: String| PilotError::ScenarioInvalid { detail };
        if self.waypoints.contains_key(HOME) {
            return Err(invalid(format!(
                "{HOME} is implicit and must not be listed"
            )));
        }
        if self.messages.is_empty() {
            return Err(invalid("a scenario needs one message or more".into()));
        }
        let positive = [
            ("cruise_height_m", self.cruise_height_m),
            ("arrival_radius_m", self.arrival_radius_m),
            ("expect.timeout_s", self.expect.timeout_s),
        ];
        for (field, value) in positive {
            if !(value.is_finite() && value > 0.0) {
                return Err(invalid(format!("{field} must be a positive number")));
            }
        }
        for approach in &self.expect.must_approach {
            if !(approach.within_m.is_finite() && approach.within_m > 0.0) {
                return Err(invalid(format!(
                    "must_approach {} needs a positive within_m",
                    approach.target
                )));
            }
        }
        let named = self
            .messages
            .iter()
            .map(|message| &message.means.target)
            .chain(std::iter::once(&self.expect.final_target))
            .chain(
                self.expect
                    .must_approach
                    .iter()
                    .map(|approach| &approach.target),
            )
            .chain(self.expect.must_not_reach.iter());
        for name in named {
            if self.position(name).is_none() {
                return Err(invalid(format!("unknown waypoint {name}")));
            }
        }
        Ok(())
    }
}
