//! Deriving every actuator decision one stream states.
//!
//! An eligible command is scaled and then either accepted or replaced by the
//! last accepted command; which of the two is a seeded decision the
//! declaration fixes. A bypassed command reaches the actuator exactly as the
//! controller wrote it, so a safety command that arrives scaled or held is a
//! command the controller never sent.

use flight_tune::{
    Digest, ExecutedActuatorApplication, ExecutedEligibility, ExecutedSample,
    ExecutedUncertaintyDeclaration,
};

use super::derivation;
use crate::{FeedbackError, error::invalid};

const ACTUATOR_LANE_COUNT: usize = 16;

type LaneBits = [u32; ACTUATOR_LANE_COUNT];

/// The open command hold and the last command it may replay.
pub(super) struct ActuatorState {
    last_accepted: Option<LaneBits>,
    interval: Option<OpenInterval>,
}

struct OpenInterval {
    identity: Digest,
    epoch: u64,
    index: u64,
    position: u32,
    decisions: Vec<bool>,
}

impl ActuatorState {
    pub(super) const fn new() -> Self {
        Self {
            last_accepted: None,
            interval: None,
        }
    }

    /// Reports whether a command hold can still replay a command.
    pub(super) const fn holds_a_command(&self) -> bool {
        self.interval.is_some()
    }

    /// Derives every actuator decision one sample states.
    ///
    /// # Errors
    ///
    /// Returns [`FeedbackError`] when a stated value is not the derived one.
    pub(super) fn accept(
        &mut self,
        declaration: &ExecutedUncertaintyDeclaration,
        sample: &ExecutedSample,
    ) -> Result<bool, FeedbackError> {
        let Some(actuator) = sample.actuator else {
            return Ok(false);
        };
        if usize::from(actuator.lane_count) > ACTUATOR_LANE_COUNT || actuator.lane_count == 0 {
            return Err(invalid("an actuator sample states no active lane"));
        }
        match actuator.eligibility {
            ExecutedEligibility::Bypass(_) => {
                if actuator.authority_scaled_lane_bits != actuator.requested_lane_bits
                    || actuator.effective_lane_bits != actuator.requested_lane_bits
                    || actuator.selected_hold
                    || actuator.applied_hold
                {
                    return Err(invalid(
                        "a bypassed command did not reach the actuator as the controller wrote it",
                    ));
                }
                self.last_accepted = None;
                self.interval = None;
                Ok(false)
            }
            ExecutedEligibility::Eligible => self.eligible(declaration, sample, &actuator),
        }
    }

    fn eligible(
        &mut self,
        declaration: &ExecutedUncertaintyDeclaration,
        sample: &ExecutedSample,
        actuator: &ExecutedActuatorApplication,
    ) -> Result<bool, FeedbackError> {
        let scaled = require_scaled(declaration, actuator)?;
        let Some(hold) = declaration.command_hold else {
            return self.untouched(actuator).map(|()| scaled);
        };
        if self.last_accepted.is_none() {
            return self.prime(actuator).map(|()| scaled);
        }
        if actuator.prime {
            return Err(invalid("a command hold primed its history a second time"));
        }
        let decisions = self.open(
            declaration,
            sample,
            actuator,
            hold.fraction_basis_points,
            hold.decision_interval_samples,
        )?;
        self.decide(actuator, &decisions)?;
        self.advance(actuator, hold.decision_interval_samples)?;
        Ok(scaled)
    }

    fn untouched(&mut self, actuator: &ExecutedActuatorApplication) -> Result<(), FeedbackError> {
        if actuator.selected_hold
            || actuator.applied_hold
            || actuator.prime
            || actuator.interval_identity.is_some()
            || actuator.effective_lane_bits != actuator.authority_scaled_lane_bits
        {
            return Err(invalid("an undeclared command hold was applied"));
        }
        self.last_accepted = Some(actuator.effective_lane_bits);
        Ok(())
    }

    fn prime(&mut self, actuator: &ExecutedActuatorApplication) -> Result<(), FeedbackError> {
        if !actuator.prime
            || actuator.selected_hold
            || actuator.applied_hold
            || actuator.interval_identity.is_some()
            || actuator.effective_lane_bits != actuator.authority_scaled_lane_bits
        {
            return Err(invalid(
                "the first eligible command of an epoch did not prime the hold history",
            ));
        }
        self.last_accepted = Some(actuator.effective_lane_bits);
        Ok(())
    }

    fn open(
        &mut self,
        declaration: &ExecutedUncertaintyDeclaration,
        sample: &ExecutedSample,
        actuator: &ExecutedActuatorApplication,
        fraction_basis_points: u16,
        decision_interval_samples: u32,
    ) -> Result<Vec<bool>, FeedbackError> {
        let (epoch, index, position, identity) = stated(actuator)?;
        if let Some(open) = &self.interval
            && open.epoch == epoch
            && open.index == index
        {
            if open.position != position || open.identity != identity {
                return Err(invalid("a command hold interval lost its own position"));
            }
            return Ok(open.decisions.clone());
        }
        if position != 0 {
            return Err(invalid(
                "a command hold interval started away from its first position",
            ));
        }
        let first_sequence = sample.global_sample_sequence;
        if derivation::interval_identity(
            declaration.condition_digest,
            declaration.run_seed,
            epoch,
            index,
            first_sequence,
        ) != identity
        {
            return Err(invalid(
                "a command hold interval does not carry its derived identity",
            ));
        }
        let decisions = derivation::hold_schedule(
            declaration.condition_digest,
            declaration.run_seed,
            epoch,
            index,
            first_sequence,
            fraction_basis_points,
            decision_interval_samples,
        )?;
        self.interval = Some(OpenInterval {
            identity,
            epoch,
            index,
            position,
            decisions: decisions.clone(),
        });
        Ok(decisions)
    }

    fn decide(
        &mut self,
        actuator: &ExecutedActuatorApplication,
        decisions: &[bool],
    ) -> Result<(), FeedbackError> {
        let position = actuator
            .interval_position
            .ok_or_else(|| invalid("a command hold states no interval position"))?;
        let selected = decisions
            .get(usize::try_from(position).unwrap_or(usize::MAX))
            .copied()
            .ok_or_else(|| invalid("a command hold reached a position it never scheduled"))?;
        if selected != actuator.selected_hold || selected != actuator.applied_hold {
            return Err(invalid(
                "a command hold decision is not the one its schedule states",
            ));
        }
        let accepted = self
            .last_accepted
            .ok_or_else(|| invalid("a command hold has no accepted command to replay"))?;
        let required = if selected {
            accepted
        } else {
            actuator.authority_scaled_lane_bits
        };
        if actuator.effective_lane_bits != required {
            return Err(invalid(
                "a held command did not replay the last accepted command",
            ));
        }
        if !selected {
            self.last_accepted = Some(actuator.effective_lane_bits);
        }
        Ok(())
    }

    fn advance(
        &mut self,
        actuator: &ExecutedActuatorApplication,
        interval_samples: u32,
    ) -> Result<(), FeedbackError> {
        let Some(open) = &mut self.interval else {
            return Err(invalid("a command hold advanced no interval"));
        };
        let next = open.position.wrapping_add(1);
        let complete = next >= interval_samples;
        if complete != actuator.interval_complete {
            return Err(invalid(
                "a command hold interval did not end where it was scheduled to",
            ));
        }
        if complete {
            self.interval = None;
        } else {
            open.position = next;
        }
        Ok(())
    }
}

/// Requires every active lane to carry the declared authority scale.
fn require_scaled(
    declaration: &ExecutedUncertaintyDeclaration,
    actuator: &ExecutedActuatorApplication,
) -> Result<bool, FeedbackError> {
    let active = usize::from(actuator.lane_count);
    let mut changed = false;
    for lane in 0..ACTUATOR_LANE_COUNT {
        let requested = actuator.requested_lane_bits[lane];
        let required = if lane < active {
            derivation::scaled_authority(requested, declaration.authority_scale_basis_points)
        } else {
            requested
        };
        if actuator.authority_scaled_lane_bits[lane] != required {
            return Err(invalid(
                "an actuator lane does not carry the declared authority scale",
            ));
        }
        changed |= required != requested;
    }
    Ok(changed)
}

fn stated(
    actuator: &ExecutedActuatorApplication,
) -> Result<(u64, u64, u32, Digest), FeedbackError> {
    match (
        actuator.interval_epoch,
        actuator.interval_index,
        actuator.interval_position,
        actuator.interval_identity,
    ) {
        (Some(epoch), Some(index), Some(position), Some(identity)) => {
            Ok((epoch, index, position, identity))
        }
        _ => Err(invalid(
            "an eligible command states no command hold interval",
        )),
    }
}
