//! The scenario document: the chart, scripted operator messages, and the
//! result that the verifier holds the flight to.
//!
//! The expected result is in this document, written by the scenario author.
//! The verifier reads it from here and never from a model. A wrong directive
//! then shows as a failed run and cannot grade itself.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::capability::{FlightEnvelope, NumberRange};
use crate::directive::{Arrival, Directive, DirectiveKind};
use crate::error::AgentError;

/// The name that every scenario gives the launch point.
pub const HOME: &str = "HOME";
const DOCUMENT: &str = "scenario";
/// Offsets closer than this ratio to a diagonal read as a diagonal.
const DIAGONAL_RATIO: f64 = 2.0;

/// A horizontal position in metres north and east of the launch point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    /// Metres north of the launch point.
    pub north_m: f64,
    /// Metres east of the launch point.
    pub east_m: f64,
}

/// A named sequence of fixes with an end behaviour. A client takes it from
/// navigation data; a scenario states it directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Procedure {
    /// The fixes in the sequence that the vehicle flies them.
    pub fixes: Vec<String>,
    /// What the vehicle does at the last fix.
    pub then: Arrival,
}

/// The condition that releases a scripted operator message.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "when", rename_all = "snake_case")]
pub enum Trigger {
    /// Released as soon as control is held.
    Start,
    /// Released after the vehicle flies the newest directive for this many
    /// seconds without a break.
    FlyingFor {
        /// Continuous flying time in seconds.
        seconds: f64,
    },
}

/// One scripted operator message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperatorMessage {
    /// The text given to the model, word for word.
    pub text: String,
    /// The release condition.
    pub trigger: Trigger,
    /// The directive that the author means. It grades the model in the run
    /// record and has no effect on the flight.
    pub means: Directive,
}

/// A condition that truth must show before the end state counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
pub enum Checkpoint {
    /// Truth comes this close to a fix.
    Approach {
        /// The fix name.
        fix: String,
        /// The horizontal distance that satisfies the checkpoint, in metres.
        within_m: f64,
    },
    /// Truth holds a heading.
    HeadingHeld {
        /// Degrees true.
        degrees: f64,
        /// The permitted difference in degrees.
        tolerance_deg: f64,
        /// How long truth must hold it, in seconds.
        for_s: f64,
    },
    /// Truth holds a height.
    HeightHeld {
        /// Metres above the launch point.
        height_m: f64,
        /// The permitted difference in metres.
        tolerance_m: f64,
        /// How long truth must hold it, in seconds.
        for_s: f64,
    },
}

/// How the flight must end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EndState {
    /// On the ground at a fix, at rest and not armed.
    LandedAt {
        /// The fix name.
        fix: String,
    },
    /// In the air at a fix, at the cruise height.
    HoldingAt {
        /// The fix name.
        fix: String,
    },
    /// The checkpoints are the whole result.
    CheckpointsOnly,
}

/// The result that the verifier holds the flight to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expectation {
    /// Conditions that truth must show, in this sequence, before the end
    /// state counts. Without them a vehicle that goes up and comes down at
    /// the launch point would pass a flight that ends at `HOME`.
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    /// How the flight must end.
    pub end: EndState,
    /// Fixes that truth must stay away from, by the arrival radius.
    #[serde(default)]
    pub must_not_reach: Vec<String>,
    /// Seconds from the first message to the verdict deadline.
    pub timeout_s: f64,
}

/// A complete scenario document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Stable identifier, copied into the run record.
    pub id: String,
    /// Named fixes. `HOME` is implicit at the origin.
    pub fixes: BTreeMap<String, Fix>,
    /// Named procedures.
    #[serde(default)]
    pub procedures: BTreeMap<String, Procedure>,
    /// Height above the launch point for the cruise, in metres.
    pub cruise_height_m: f64,
    /// Horizontal distance that counts as arrival, in metres.
    pub arrival_radius_m: f64,
    /// The largest permitted distance from the launch point, in metres. A
    /// heading has no end, so the executor holds position at this range.
    pub max_range_m: f64,
    /// The permitted height for an `altitude` directive.
    pub height_m: NumberRange,
    /// Scripted operator messages, in release sequence.
    #[serde(default)]
    pub messages: Vec<OperatorMessage>,
    /// The result that the verifier holds the flight to.
    pub expect: Expectation,
}

impl Scenario {
    /// Parses and validates a scenario document.
    pub fn parse(text: &str) -> Result<Self, AgentError> {
        let scenario: Self = serde_json::from_str(text).map_err(|source| AgentError::Parse {
            document: DOCUMENT,
            source,
        })?;
        scenario.validate()?;
        Ok(scenario)
    }

    /// The position of a named fix. `HOME` is the origin.
    #[must_use]
    pub fn position(&self, name: &str) -> Option<Fix> {
        if name == HOME {
            return Some(Fix {
                north_m: 0.0,
                east_m: 0.0,
            });
        }
        self.fixes.get(name).copied()
    }

    /// What the agent can fly in this scenario. `speed_mps` comes from the
    /// vehicle advertisement, which the scenario does not know.
    #[must_use]
    pub fn envelope(&self, speed_mps: NumberRange) -> FlightEnvelope {
        let mut fixes: Vec<String> = self.fixes.keys().cloned().collect();
        fixes.push(HOME.to_owned());
        FlightEnvelope {
            kinds: DirectiveKind::ALL.to_vec(),
            fixes,
            procedures: self.procedures.keys().cloned().collect(),
            height_m: self.height_m,
            speed_mps,
        }
    }

    /// The legend for the model: where each fix is and what each procedure
    /// is, in short sentences.
    #[must_use]
    pub fn legend(&self) -> String {
        let mut text = format!("FIXES: {HOME} is the launch point and base.");
        for (name, fix) in &self.fixes {
            let distance = fix.north_m.hypot(fix.east_m).round();
            let direction = compass(fix.north_m, fix.east_m);
            text.push_str(&format!(" {name} is {distance} m {direction} of {HOME}."));
        }
        if !self.procedures.is_empty() {
            text.push_str(" PROCEDURES:");
            for (name, procedure) in &self.procedures {
                let end = match procedure.then {
                    Arrival::Land => "land",
                    Arrival::Hold => "hold",
                };
                let route = procedure.fixes.join(", ");
                text.push_str(&format!(" {name} goes {route}, then {end}."));
            }
        }
        text
    }

    fn validate(&self) -> Result<(), AgentError> {
        let invalid = |detail: String| AgentError::Invalid {
            document: DOCUMENT,
            detail,
        };
        if self.fixes.contains_key(HOME) {
            return Err(invalid(format!(
                "{HOME} is implicit and must not be listed"
            )));
        }
        let positive = [
            ("cruise_height_m", self.cruise_height_m),
            ("arrival_radius_m", self.arrival_radius_m),
            ("max_range_m", self.max_range_m),
            ("expect.timeout_s", self.expect.timeout_s),
        ];
        for (field, value) in positive {
            if !(value.is_finite() && value > 0.0) {
                return Err(invalid(format!("{field} must be a positive number")));
            }
        }
        if !self.height_m.admits(self.cruise_height_m) {
            return Err(invalid("cruise_height_m is outside height_m".to_owned()));
        }
        for name in self.named_fixes() {
            if self.position(name).is_none() {
                return Err(invalid(format!("unknown fix {name}")));
            }
        }
        for (name, procedure) in &self.procedures {
            if procedure.fixes.is_empty() {
                return Err(invalid(format!("procedure {name} has no fix")));
            }
        }
        Ok(())
    }

    /// Every fix name that the document refers to.
    fn named_fixes(&self) -> Vec<&String> {
        let mut names: Vec<&String> = self
            .procedures
            .values()
            .flat_map(|procedure| procedure.fixes.iter())
            .chain(self.expect.must_not_reach.iter())
            .collect();
        for checkpoint in &self.expect.checkpoints {
            if let Checkpoint::Approach { fix, .. } = checkpoint {
                names.push(fix);
            }
        }
        match &self.expect.end {
            EndState::LandedAt { fix } | EndState::HoldingAt { fix } => names.push(fix),
            EndState::CheckpointsOnly => {}
        }
        names
    }
}

fn compass(north_m: f64, east_m: f64) -> &'static str {
    let (north, east) = (north_m.abs(), east_m.abs());
    let northward = north_m >= 0.0;
    let eastward = east_m >= 0.0;
    if north >= DIAGONAL_RATIO * east {
        if northward { "north" } else { "south" }
    } else if east >= DIAGONAL_RATIO * north {
        if eastward { "east" } else { "west" }
    } else {
        match (northward, eastward) {
            (true, true) => "north-east",
            (true, false) => "north-west",
            (false, true) => "south-east",
            (false, false) => "south-west",
        }
    }
}

#[cfg(test)]
mod tests;
