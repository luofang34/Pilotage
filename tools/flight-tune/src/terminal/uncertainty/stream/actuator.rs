//! Deriving what one declaration required of every actuator command.
//!
//! An eligible command is scaled and then either accepted or replaced by the
//! last accepted command, and which of the two is a seeded decision the
//! declaration fixes. A bypassed command reaches the actuator exactly as the
//! controller wrote it: no scale and no hold, because a safety command that
//! arrives changed is not the command the controller sent.

use super::super::super::invalid_terminal;
use super::super::sample::{
    EXECUTED_ACTUATOR_LANE_COUNT, ExecutedActuatorApplication, ExecutedEligibility, ExecutedSample,
};
use super::super::{ExecutedUncertaintyDeclaration, derivation};
use crate::{Digest, TuneError};

type LaneBits = [u32; EXECUTED_ACTUATOR_LANE_COUNT];

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
}

/// Derives every actuator decision one sample states.
///
/// Returns whether the declared authority scale changed a commanded lane.
pub(super) fn verify(
    state: &mut ActuatorState,
    declaration: &ExecutedUncertaintyDeclaration,
    sample: &ExecutedSample,
) -> Result<bool, TuneError> {
    let Some(actuator) = sample.actuator else {
        return Ok(false);
    };
    match actuator.eligibility {
        ExecutedEligibility::Bypass(_) => {
            require_untouched(&actuator)?;
            state.last_accepted = None;
            state.interval = None;
            Ok(false)
        }
        ExecutedEligibility::Eligible => verify_eligible(state, declaration, sample, &actuator),
    }
}

/// Requires a bypassed command to reach the actuator as it was written.
fn require_untouched(actuator: &ExecutedActuatorApplication) -> Result<(), TuneError> {
    if actuator.authority_scaled_lane_bits != actuator.requested_lane_bits
        || actuator.effective_lane_bits != actuator.requested_lane_bits
    {
        return Err(invalid_terminal(
            "a bypassed command did not reach the actuator as the controller wrote it",
        ));
    }
    Ok(())
}

fn verify_eligible(
    state: &mut ActuatorState,
    declaration: &ExecutedUncertaintyDeclaration,
    sample: &ExecutedSample,
    actuator: &ExecutedActuatorApplication,
) -> Result<bool, TuneError> {
    let scaled = require_scaled(declaration, actuator)?;
    let Some(hold) = declaration.command_hold else {
        return require_no_hold(state, actuator).map(|()| scaled);
    };
    if state.last_accepted.is_none() {
        return require_prime(state, actuator).map(|()| scaled);
    }
    if actuator.prime {
        return Err(invalid_terminal(
            "a command hold primed its history a second time",
        ));
    }
    let decisions = open_interval(state, declaration, sample, actuator, hold)?;
    require_decision(state, actuator, &decisions)?;
    advance(state, actuator, hold.decision_interval_samples)?;
    Ok(scaled)
}

/// Requires every active lane to carry the declared authority scale.
fn require_scaled(
    declaration: &ExecutedUncertaintyDeclaration,
    actuator: &ExecutedActuatorApplication,
) -> Result<bool, TuneError> {
    let active = usize::from(actuator.lane_count);
    let mut changed = false;
    for lane in 0..EXECUTED_ACTUATOR_LANE_COUNT {
        let requested = actuator.requested_lane_bits[lane];
        let required = if lane < active {
            derivation::scaled_authority(requested, declaration.authority_scale_basis_points)
        } else {
            requested
        };
        if actuator.authority_scaled_lane_bits[lane] != required {
            return Err(invalid_terminal(
                "an actuator lane does not carry the declared authority scale",
            ));
        }
        changed |= required != requested;
    }
    Ok(changed)
}

/// Requires a run that declared no hold to command the scaled value.
fn require_no_hold(
    state: &mut ActuatorState,
    actuator: &ExecutedActuatorApplication,
) -> Result<(), TuneError> {
    if actuator.selected_hold || actuator.applied_hold || actuator.prime {
        return Err(invalid_terminal("an undeclared command hold was applied"));
    }
    if actuator.interval_identity.is_some() {
        return Err(invalid_terminal(
            "an undeclared command hold opened an interval",
        ));
    }
    if actuator.effective_lane_bits != actuator.authority_scaled_lane_bits {
        return Err(invalid_terminal(
            "an eligible command did not reach the actuator scaled",
        ));
    }
    state.last_accepted = Some(actuator.effective_lane_bits);
    Ok(())
}

/// Requires the first eligible command of an epoch to only prime the history.
fn require_prime(
    state: &mut ActuatorState,
    actuator: &ExecutedActuatorApplication,
) -> Result<(), TuneError> {
    if !actuator.prime || actuator.selected_hold || actuator.applied_hold {
        return Err(invalid_terminal(
            "the first eligible command of an epoch did not prime the hold history",
        ));
    }
    if actuator.interval_identity.is_some() {
        return Err(invalid_terminal("a priming command opened an interval"));
    }
    if actuator.effective_lane_bits != actuator.authority_scaled_lane_bits {
        return Err(invalid_terminal(
            "a priming command did not reach the actuator scaled",
        ));
    }
    state.last_accepted = Some(actuator.effective_lane_bits);
    Ok(())
}

/// Derives the schedule the stated interval identity must carry.
fn open_interval(
    state: &mut ActuatorState,
    declaration: &ExecutedUncertaintyDeclaration,
    sample: &ExecutedSample,
    actuator: &ExecutedActuatorApplication,
    hold: super::super::DeclaredCommandHold,
) -> Result<Vec<bool>, TuneError> {
    let (epoch, index, position, identity) = stated_interval(actuator)?;
    if let Some(open) = &state.interval
        && open.epoch == epoch
        && open.index == index
    {
        if open.position != position || open.identity != identity {
            return Err(invalid_terminal(
                "a command hold interval lost its own position",
            ));
        }
        return Ok(open.decisions.clone());
    }
    if position != 0 {
        return Err(invalid_terminal(
            "a command hold interval started away from its first position",
        ));
    }
    let first_sequence = sample.global_sample_sequence;
    let derived = derivation::interval_identity(
        declaration.condition_digest,
        declaration.run_seed,
        epoch,
        index,
        first_sequence,
    );
    if derived != identity {
        return Err(invalid_terminal(
            "a command hold interval does not carry its derived identity",
        ));
    }
    let decisions = derivation::hold_schedule(
        declaration.condition_digest,
        declaration.run_seed,
        epoch,
        index,
        first_sequence,
        hold,
    )?;
    state.interval = Some(OpenInterval {
        identity,
        epoch,
        index,
        position,
        decisions: decisions.clone(),
    });
    Ok(decisions)
}

/// Requires the stated hold decision to be the one the schedule holds.
fn require_decision(
    state: &mut ActuatorState,
    actuator: &ExecutedActuatorApplication,
    decisions: &[bool],
) -> Result<(), TuneError> {
    let position = actuator
        .interval_position
        .ok_or_else(|| invalid_terminal("a command hold states no interval position"))?;
    let selected = decisions
        .get(usize::try_from(position).unwrap_or(usize::MAX))
        .copied()
        .ok_or_else(|| invalid_terminal("a command hold reached a position it never scheduled"))?;
    if selected != actuator.selected_hold || selected != actuator.applied_hold {
        return Err(invalid_terminal(
            "a command hold decision is not the one its schedule states",
        ));
    }
    let accepted = state
        .last_accepted
        .ok_or_else(|| invalid_terminal("a command hold has no accepted command to replay"))?;
    let required = if selected {
        accepted
    } else {
        actuator.authority_scaled_lane_bits
    };
    if actuator.effective_lane_bits != required {
        return Err(invalid_terminal(
            "a held command did not replay the last accepted command",
        ));
    }
    if !selected {
        state.last_accepted = Some(actuator.effective_lane_bits);
    }
    Ok(())
}

/// Advances or closes the open interval after one counted decision.
fn advance(
    state: &mut ActuatorState,
    actuator: &ExecutedActuatorApplication,
    interval_samples: u32,
) -> Result<(), TuneError> {
    let Some(open) = &mut state.interval else {
        return Err(invalid_terminal("a command hold advanced no interval"));
    };
    let next = open.position.wrapping_add(1);
    let complete = next >= interval_samples;
    if complete != actuator.interval_complete {
        return Err(invalid_terminal(
            "a command hold interval did not end where it was scheduled to",
        ));
    }
    if complete {
        state.interval = None;
    } else {
        open.position = next;
    }
    Ok(())
}

fn stated_interval(
    actuator: &ExecutedActuatorApplication,
) -> Result<(u64, u64, u32, Digest), TuneError> {
    match (
        actuator.interval_epoch,
        actuator.interval_index,
        actuator.interval_position,
        actuator.interval_identity,
    ) {
        (Some(epoch), Some(index), Some(position), Some(identity)) => {
            Ok((epoch, index, position, identity))
        }
        _ => Err(invalid_terminal(
            "an eligible command states no command hold interval",
        )),
    }
}
