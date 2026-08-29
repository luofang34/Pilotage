//! The declared scenario matrix a vehicle campaign flies.
//!
//! The matrix is a declaration, not a directory listing. It states which
//! stimuli exist, which uncertainty factors exist, and the rule that pairs
//! them, and the generated corpus is then checked against it. A checker that
//! read the directory as its own answer would report complete coverage of
//! whatever happened to be on disk.
//!
//! Every uncertainty factor here carries its exact executable value. A factor
//! declared as a label or a Boolean could be counted as covered by an artifact
//! that requests nothing, which is the coverage this layer exists to refuse.
//!
//! SIM / NOT FOR FLIGHT.

use pilotage_trial::{ControlChannel, ControlFamily};

mod matrix;
mod projection;
mod targets;

#[cfg(test)]
#[path = "scenario/tests.rs"]
mod tests;

pub use matrix::{LoadedCell, LoadedMatrix, MatrixReport};
pub use projection::{condition_path, scenario_path};
pub use targets::{alia250_matrix_response_targets, matrix_mission};

/// The isolated partition one cell belongs to.
///
/// A candidate fitted to a training disturbance meets a different one on the
/// run that decides what ships, so each partition carries its own seed stream
/// and its own artifact identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatrixPartition {
    /// The partition adaptive search reads.
    Training,
    /// The hidden partition the one promotion decision reads.
    Promotion,
    /// The hidden partition the final release decision reads.
    FinalQualification,
}

impl MatrixPartition {
    /// Every partition, in declaration order.
    pub const ALL: [Self; 3] = [Self::Training, Self::Promotion, Self::FinalQualification];

    /// The stable partition name that a generated path carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Training => "training",
            Self::Promotion => "promotion",
            Self::FinalQualification => "final",
        }
    }
}

/// One declared stimulus and the physical command it requests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatrixStimulus {
    /// The stable stimulus name a generated path carries.
    pub id: &'static str,
    /// The physical control family the stimulus commands.
    pub family: ControlFamily,
    /// The control channel the stimulus commands.
    pub channel: ControlChannel,
    /// The stable envelope identifier.
    pub envelope_id: &'static str,
    /// The physical value at normalized plus one.
    pub positive_endpoint: f64,
    /// The normalized value the trial holds.
    pub normalized_value: f64,
}

impl MatrixStimulus {
    /// The physical value this stimulus asks the vehicle to produce.
    ///
    /// For a direct family the affine map is exact, so this is the command.
    /// For an operator family the envelope bounds the candidate curve, so this
    /// is the most the stick can be worth.
    #[must_use]
    pub fn physical_target(&self) -> f64 {
        self.positive_endpoint * self.normalized_value
    }
}

/// One declared uncertainty factor, with the exact value an artifact carries.
///
/// The coverage rule compares these numbers against the decoded artifact. A
/// factor whose declaration held only a name could be satisfied by an artifact
/// that requested nothing at all.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UncertaintyFactor {
    /// No perturbation of any kind.
    Calm,
    /// A constant wind from a stated direction.
    SteadyWind {
        /// The wind speed in meters per second.
        speed_mps: f64,
        /// The true direction the wind comes from, in degrees.
        direction_deg: f64,
    },
    /// One gust that rises, holds, and releases.
    Gust {
        /// The gust speed in meters per second.
        speed_mps: f64,
        /// The hold duration in simulator nanoseconds.
        hold_ns: u64,
    },
    /// A scaled actuator authority, in basis points of nominal.
    ActuatorAuthority {
        /// The requested scale in basis points.
        basis_points: u16,
    },
    /// A scaled hover feed-forward force, in basis points of nominal.
    HoverTrim {
        /// The requested scale in basis points.
        basis_points: u16,
    },
    /// Bounded deterministic noise on the declared sensor lanes.
    SensorNoise {
        /// The number of lanes the request declares.
        lanes: usize,
    },
    /// A seeded additional update delay.
    TimingJitter {
        /// The largest additional delay in nanoseconds.
        maximum_delay_ns: u64,
        /// The interval between deterministic delay values, in nanoseconds.
        interval_ns: u64,
    },
    /// A fixed estimate age.
    AddedDelay {
        /// The estimate age in nanoseconds.
        estimate_delay_ns: u64,
    },
    /// A seeded zero-order-hold command-loss policy.
    CommandLoss {
        /// The held fraction in basis points.
        fraction_basis_points: u16,
        /// The number of eligible commands in one decision interval.
        decision_interval_samples: u32,
    },
}

/// One declared condition: a name and the factor its artifact requests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatrixCondition {
    /// The stable condition name a generated path carries.
    pub id: &'static str,
    /// The exact executable value the artifact has to carry.
    pub factor: UncertaintyFactor,
}

/// One declared cell of the matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatrixCell {
    /// The partition this cell belongs to.
    pub partition: MatrixPartition,
    /// The stimulus this cell commands.
    pub stimulus: MatrixStimulus,
    /// The condition this cell applies.
    pub condition: MatrixCondition,
}

/// The complete declaration of one vehicle's scenario matrix.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioMatrix {
    /// The stable matrix identifier.
    pub id: &'static str,
    /// Every declared stimulus, in declaration order.
    pub stimuli: &'static [MatrixStimulus],
    /// Every declared condition, in declaration order. The first is calm.
    pub conditions: &'static [MatrixCondition],
    /// The stimulus of each control family that carries every factor.
    pub family_representatives: &'static [&'static str],
}

impl ScenarioMatrix {
    /// Every cell this matrix declares, in one stable order.
    ///
    /// Each stimulus flies calm in every partition, and each uncertainty
    /// factor flies on one representative of each control family in every
    /// partition. The count follows from those two rules, so a missing or an
    /// extra artifact is a difference from the declaration rather than from a
    /// number someone wrote down.
    #[must_use]
    pub fn cells(&self) -> Vec<MatrixCell> {
        let mut cells = Vec::with_capacity(self.expected_cell_count());
        for partition in MatrixPartition::ALL {
            for stimulus in self.stimuli {
                cells.push(MatrixCell {
                    partition,
                    stimulus: *stimulus,
                    condition: self.conditions[0],
                });
            }
            for condition in &self.conditions[1..] {
                for name in self.family_representatives {
                    if let Some(stimulus) = self.stimulus(name) {
                        cells.push(MatrixCell {
                            partition,
                            stimulus,
                            condition: *condition,
                        });
                    }
                }
            }
        }
        cells
    }

    /// The number of cells the declaration produces.
    #[must_use]
    pub fn expected_cell_count(&self) -> usize {
        let calm = self.stimuli.len();
        let uncertainty = self
            .conditions
            .len()
            .saturating_sub(1)
            .saturating_mul(self.family_representatives.len());
        MatrixPartition::ALL
            .len()
            .saturating_mul(calm.saturating_add(uncertainty))
    }

    /// The number of generated artifacts the declaration produces.
    ///
    /// Each cell states one scenario and one condition, so the file count is
    /// derived from the declaration rather than counted from the directory.
    #[must_use]
    pub fn expected_document_count(&self) -> usize {
        self.expected_cell_count().saturating_mul(2)
    }

    /// The declared stimulus with one name.
    #[must_use]
    pub fn stimulus(&self, id: &str) -> Option<MatrixStimulus> {
        self.stimuli.iter().copied().find(|entry| entry.id == id)
    }

    /// Every uncertainty factor the matrix declares, without calm.
    #[must_use]
    pub fn uncertainty_conditions(&self) -> &[MatrixCondition] {
        &self.conditions[1..]
    }
}

mod alia250;

pub use alia250::ALIA250_MATRIX;
