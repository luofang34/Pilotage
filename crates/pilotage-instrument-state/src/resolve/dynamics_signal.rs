//! Resolution of the typed turn and slip/skid group (DYN-01).

use crate::aircraft::AircraftState;
use crate::dynamics::TurnBasis;
use crate::signal::{FreshnessPolicy, Sig, SignalStatus};
use crate::validate::StateIntegrity;

use super::{ResolvedTurn, Trust, finite, group_freshness};

/// Resolves the typed turn indication. Only the dynamics group feeds
/// it: absence is `Missing` (body rates are not consulted), an unknown
/// basis fails, and the value is retained unclamped for monitoring.
pub(super) fn turn_resolved(
    state: &AircraftState,
    policy: &FreshnessPolicy,
    trust: &Trust,
    integrity: &StateIntegrity,
) -> ResolvedTurn {
    let sample = state.dynamics.data.and_then(|dynamics| dynamics.turn);
    let has = sample.is_some();
    let fresh = group_freshness(policy, state.dynamics.data.is_some(), state.dynamics.age_ms);
    let status = if !has {
        SignalStatus::Missing
    } else {
        trust.fold(true, fresh, integrity.dynamics, state.valid.turn)
    };
    let sample = sample.unwrap_or(crate::dynamics::TurnSample {
        rate_rps: 0.0,
        basis: TurnBasis::Unknown,
    });
    let status = if sample.basis == TurnBasis::Unknown && has {
        SignalStatus::Failed
    } else {
        status
    };
    ResolvedTurn {
        rate_rps: finite(Sig::with_status(sample.rate_rps, status)),
        basis: sample.basis,
    }
}

/// Resolves the slip/skid lateral force. Missing stays missing — a
/// centered ball is a claim of coordination nobody made.
pub(super) fn slip_resolved(
    state: &AircraftState,
    policy: &FreshnessPolicy,
    trust: &Trust,
    integrity: &StateIntegrity,
) -> Sig<f32> {
    let value = state
        .dynamics
        .data
        .and_then(|dynamics| dynamics.lateral_mps2);
    let fresh = group_freshness(policy, state.dynamics.data.is_some(), state.dynamics.age_ms);
    match value {
        Some(lateral) => finite(Sig::with_status(
            lateral,
            trust.fold(true, fresh, integrity.dynamics, state.valid.slip),
        )),
        None => Sig::missing(),
    }
}
