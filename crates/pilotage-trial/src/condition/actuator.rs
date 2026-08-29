//! Deterministic actuator perturbation contracts.
//!
//! An actuator request acts after the controller command and before the
//! simulator actuator write. It must not change the test stimulus.

use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

use crate::{BackendCapability, Digest, ValidationError};

#[cfg(test)]
mod tests;

const BASIS_POINTS_NOMINAL: u16 = 10_000;
const MIN_AUTHORITY_BASIS_POINTS: u16 = 5_000;
const MAX_AUTHORITY_BASIS_POINTS: u16 = 15_000;
const MAX_COMMAND_HOLD_BASIS_POINTS: u16 = 1_000;
const MAX_DECISION_INTERVAL_SAMPLES: u32 = 10_000;
const COMMAND_HOLD_DOMAIN: &[u8] = b"pilotage-command-hold-v1";

/// A deterministic command-loss policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandLossPolicy {
    /// Accept each eligible command.
    None {},
    /// Hold an exact seeded set in each complete decision interval.
    SeededZeroOrderHold {
        /// The held fraction in basis points.
        fraction_basis_points: u16,
        /// The number of eligible commands in one decision interval.
        decision_interval_samples: u32,
    },
}

/// Actuator perturbations for one condition set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActuatorCondition {
    /// Scale for eligible actuator commands in basis points.
    pub authority_scale_basis_points: u16,
    /// The deterministic command-loss policy.
    pub command_loss: CommandLossPolicy,
}

/// One reference command-hold action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandHoldAction {
    /// Accept the current safe command.
    Accept,
    /// Keep the last accepted safe command.
    HoldLastAccepted,
}

/// The stable identity inputs for one complete command-hold interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandHoldIntervalIdentity {
    condition_digest: Digest,
    run_seed: u64,
    interval_epoch: u64,
    interval_index: u64,
    first_eligible_global_sample_sequence: u64,
}

impl CommandHoldIntervalIdentity {
    /// Creates one command-hold interval identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition digest contains only zero bytes.
    pub fn new(
        condition_digest: Digest,
        run_seed: u64,
        interval_epoch: u64,
        interval_index: u64,
        first_eligible_global_sample_sequence: u64,
    ) -> Result<Self, ValidationError> {
        if condition_digest.is_zero() {
            return Err(ValidationError::InvalidRelation {
                field: "condition_set.actuator.command_loss.condition_digest".to_owned(),
                relation: "use the canonical nonzero condition digest",
            });
        }
        Ok(Self {
            condition_digest,
            run_seed,
            interval_epoch,
            interval_index,
            first_eligible_global_sample_sequence,
        })
    }

    /// Returns the digest of the fixed-width interval identity preimage.
    ///
    /// The preimage starts with the command-hold domain and condition digest.
    /// It then has the run seed, epoch, index, and first eligible sequence.
    /// Each integer is an unsigned 64-bit little-endian value.
    #[must_use]
    pub fn digest(self) -> Digest {
        let mut hasher = Sha256::new();
        self.update_hasher(&mut hasher);
        Digest::from_bytes(hasher.finalize().into())
    }

    fn update_hasher(self, hasher: &mut Sha256) {
        hasher.update(COMMAND_HOLD_DOMAIN);
        hasher.update(self.condition_digest.as_bytes());
        hasher.update(self.run_seed.to_le_bytes());
        hasher.update(self.interval_epoch.to_le_bytes());
        hasher.update(self.interval_index.to_le_bytes());
        hasher.update(self.first_eligible_global_sample_sequence.to_le_bytes());
    }
}

impl ActuatorCondition {
    /// Returns the nominal actuator condition.
    #[must_use]
    pub const fn nominal() -> Self {
        Self {
            authority_scale_basis_points: BASIS_POINTS_NOMINAL,
            command_loss: CommandLossPolicy::None {},
        }
    }

    /// Reports whether actuator authority is nominal.
    #[must_use]
    pub const fn has_nominal_authority(self) -> bool {
        self.authority_scale_basis_points == BASIS_POINTS_NOMINAL
    }

    /// Returns the exact ratio applied to one eligible actuator lane.
    #[must_use]
    pub fn authority_scale(self) -> f64 {
        f64::from(self.authority_scale_basis_points) / f64::from(BASIS_POINTS_NOMINAL)
    }

    /// Returns the capabilities that this actuator condition needs.
    #[must_use]
    pub fn required_capabilities(self) -> Vec<BackendCapability> {
        let mut required = Vec::new();
        if !self.has_nominal_authority() {
            required.push(BackendCapability::ActuatorAuthority);
        }
        if !matches!(self.command_loss, CommandLossPolicy::None {}) {
            required.push(BackendCapability::CommandHold);
        }
        required
    }

    /// Validates the complete actuator perturbation.
    ///
    /// # Errors
    ///
    /// Returns an error when the authority scale or the command-loss policy
    /// is outside its fixed bound.
    pub fn validate(self) -> Result<(), ValidationError> {
        integer_range(
            "condition_set.actuator.authority_scale_basis_points",
            u64::from(self.authority_scale_basis_points),
            u64::from(MIN_AUTHORITY_BASIS_POINTS),
            u64::from(MAX_AUTHORITY_BASIS_POINTS),
        )?;
        self.command_loss.validate()
    }
}

impl CommandLossPolicy {
    /// Returns the exact held command count in one complete interval.
    ///
    /// # Errors
    ///
    /// Returns an error when the policy is outside its fixed bounds or does
    /// not give an exact integer count.
    pub fn exact_hold_count(self) -> Result<u32, ValidationError> {
        self.validate()?;
        let count = match self {
            Self::None {} => 0,
            Self::SeededZeroOrderHold {
                fraction_basis_points,
                decision_interval_samples,
            } => {
                let product =
                    u64::from(fraction_basis_points) * u64::from(decision_interval_samples);
                product / u64::from(BASIS_POINTS_NOMINAL)
            }
        };
        u32::try_from(count).map_err(|_| permutation_index_error())
    }

    /// Builds the stable hold decisions for one complete interval.
    ///
    /// Position zero is the second eligible ordinary command of the run. The
    /// first eligible ordinary command primes the accepted-command history
    /// outside every decision interval.
    ///
    /// # Errors
    ///
    /// Returns an error when the policy is invalid or the interval does not
    /// fit the target index type.
    pub fn decisions_for_interval(
        self,
        identity: CommandHoldIntervalIdentity,
    ) -> Result<Vec<bool>, ValidationError> {
        self.validate()?;
        let Self::SeededZeroOrderHold {
            decision_interval_samples,
            ..
        } = self
        else {
            return Ok(Vec::new());
        };
        let size = usize::try_from(decision_interval_samples).map_err(|_| index_fit_error())?;
        let mut positions = (0..size).collect::<Vec<_>>();
        permute(&mut positions, identity)?;
        let hold_count =
            usize::try_from(self.exact_hold_count()?).map_err(|_| index_fit_error())?;
        let mut decisions = vec![false; size];
        for position in positions.into_iter().take(hold_count) {
            decisions[position] = true;
        }
        Ok(decisions)
    }

    /// Returns the action for the first eligible command before interval zero.
    #[must_use]
    pub const fn prime_action(self) -> CommandHoldAction {
        CommandHoldAction::Accept
    }

    /// Resolves one selected hold against the accepted-command history.
    ///
    /// A selected hold with no accepted command still accepts, so the first
    /// eligible command of a run can never leave the actuator without a
    /// commanded value.
    #[must_use]
    pub const fn action(selected_hold: bool, has_accepted_command: bool) -> CommandHoldAction {
        if selected_hold && has_accepted_command {
            CommandHoldAction::HoldLastAccepted
        } else {
            CommandHoldAction::Accept
        }
    }

    /// Validates the command-loss policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the fraction or interval is outside its fixed
    /// bound, or when the fraction does not give an exact integer count.
    pub fn validate(self) -> Result<(), ValidationError> {
        let Self::SeededZeroOrderHold {
            fraction_basis_points,
            decision_interval_samples,
        } = self
        else {
            return Ok(());
        };
        integer_range(
            "condition_set.actuator.command_loss.fraction_basis_points",
            u64::from(fraction_basis_points),
            1,
            u64::from(MAX_COMMAND_HOLD_BASIS_POINTS),
        )?;
        integer_range(
            "condition_set.actuator.command_loss.decision_interval_samples",
            u64::from(decision_interval_samples),
            1,
            u64::from(MAX_DECISION_INTERVAL_SAMPLES),
        )?;
        let product = u64::from(fraction_basis_points) * u64::from(decision_interval_samples);
        if product % u64::from(BASIS_POINTS_NOMINAL) == 0 {
            return Ok(());
        }
        Err(ValidationError::InvalidRelation {
            field: "condition_set.actuator.command_loss".to_owned(),
            relation: "select an exact integer count in each complete decision interval",
        })
    }
}

fn permute(
    positions: &mut [usize],
    identity: CommandHoldIntervalIdentity,
) -> Result<(), ValidationError> {
    for cursor in (1..positions.len()).rev() {
        let encoded_cursor = u64::try_from(cursor).map_err(|_| index_fit_error())?;
        let value = permutation_value(identity, encoded_cursor);
        let bound = encoded_cursor.wrapping_add(1);
        let swap = usize::try_from(value % bound).map_err(|_| index_fit_error())?;
        positions.swap(cursor, swap);
    }
    Ok(())
}

fn permutation_value(identity: CommandHoldIntervalIdentity, cursor: u64) -> u64 {
    let mut hasher = Sha256::new();
    identity.update_hasher(&mut hasher);
    hasher.update(cursor.to_le_bytes());
    let bytes = hasher.finalize();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

fn permutation_index_error() -> ValidationError {
    ValidationError::InvalidRelation {
        field: "condition_set.actuator.command_loss.fraction_basis_points".to_owned(),
        relation: "give a hold count inside the fixed 32-bit interval size",
    }
}

fn index_fit_error() -> ValidationError {
    ValidationError::InvalidRelation {
        field: "condition_set.actuator.command_loss.decision_interval_samples".to_owned(),
        relation: "fit the fixed 64-bit permutation index",
    }
}

fn integer_range(
    field: &str,
    actual: u64,
    minimum: u64,
    maximum: u64,
) -> Result<(), ValidationError> {
    if (minimum..=maximum).contains(&actual) {
        return Ok(());
    }
    Err(ValidationError::OutOfRange {
        field: field.to_owned(),
        actual: actual as f64,
        minimum: minimum as f64,
        maximum: maximum as f64,
    })
}
