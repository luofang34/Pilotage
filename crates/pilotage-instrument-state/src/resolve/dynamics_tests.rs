#![allow(clippy::expect_used, clippy::panic)]

//! DYN-01 resolution proofs: the turn indication is typed, body rates
//! never substitute, slip stays missing when absent, and over-range
//! values survive unclamped for monitoring.

use super::tests::flying_state;
use crate::aircraft::{AircraftState, Stamped};
use crate::dynamics::{DynSample, TurnBasis, TurnSample};
use crate::signal::{FreshnessPolicy, SignalStatus};
use crate::{SnapshotCoherence, resolve};

fn with_dynamics(turn: Option<TurnSample>, lateral: Option<f32>) -> AircraftState {
    let mut state = flying_state();
    state.dynamics = Stamped {
        data: Some(DynSample {
            turn,
            lateral_mps2: lateral,
        }),
        age_ms: Some(20.0),
    };
    state.valid.turn = true;
    state.valid.slip = true;
    state
}

fn heading_turn(rate_rps: f32) -> Option<TurnSample> {
    Some(TurnSample {
        rate_rps,
        basis: TurnBasis::HeadingRate,
    })
}

#[test]
fn body_yaw_rate_never_reaches_the_turn_indication() {
    // A large body r at a rolled/pitched attitude with NO dynamics
    // group: the turn indication is Missing, not 0.3 rad/s.
    let mut state = flying_state();
    if let Some(att) = state.attitude.data.as_mut() {
        att.rates_rps = [0.1, 0.2, 0.3];
    }
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Missing);
    assert_eq!(p.turn.rate_rps.value, 0.0);
    assert_eq!(p.turn.basis, TurnBasis::Unknown);
}

#[test]
fn typed_turn_resolves_with_its_basis() {
    let p = resolve(
        &with_dynamics(heading_turn(0.05), None),
        &FreshnessPolicy::default(),
    );
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Valid);
    assert!((p.turn.rate_rps.value - 0.05).abs() < 1e-6);
    assert_eq!(p.turn.basis, TurnBasis::HeadingRate);
    assert_eq!(p.turn.basis.label(), "HDG");
}

#[test]
fn unknown_basis_fails_the_turn() {
    let state = with_dynamics(
        Some(TurnSample {
            rate_rps: 0.05,
            basis: TurnBasis::from_u8(9),
        }),
        None,
    );
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Failed);
}

#[test]
fn over_range_turn_is_retained_unclamped_for_monitoring() {
    // 20°/s is far past the ±3°/s display scale; the resolved value
    // keeps it exactly — only the pointer geometry saturates.
    let rate = 20.0_f32.to_radians();
    let p = resolve(
        &with_dynamics(heading_turn(rate), None),
        &FreshnessPolicy::default(),
    );
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Valid);
    assert!((p.turn.rate_rps.value - rate).abs() < 1e-6);
}

#[test]
fn missing_slip_stays_missing_and_present_slip_is_symmetric() {
    let p = resolve(
        &with_dynamics(heading_turn(0.0), None),
        &FreshnessPolicy::default(),
    );
    assert_eq!(p.slip_lat_mps2.status, SignalStatus::Missing);

    let right = resolve(
        &with_dynamics(heading_turn(0.0), Some(1.5)),
        &FreshnessPolicy::default(),
    );
    let left = resolve(
        &with_dynamics(heading_turn(0.0), Some(-1.5)),
        &FreshnessPolicy::default(),
    );
    assert_eq!(right.slip_lat_mps2.status, SignalStatus::Valid);
    assert!((right.slip_lat_mps2.value + left.slip_lat_mps2.value).abs() < 1e-6);
}

#[test]
fn declared_invalidity_and_staleness_fail_the_dynamics() {
    let mut state = with_dynamics(heading_turn(0.05), Some(0.5));
    state.valid.turn = false;
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Failed);
    assert_eq!(
        p.slip_lat_mps2.status,
        SignalStatus::Valid,
        "slip validity is independent of the turn flag"
    );

    let mut state = with_dynamics(heading_turn(0.05), Some(0.5));
    state.dynamics.age_ms = Some(1.0e7);
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Failed);
    assert_eq!(p.slip_lat_mps2.status, SignalStatus::Failed);
}

#[test]
fn coherence_skew_degrades_present_dynamics() {
    let mut state = with_dynamics(heading_turn(0.05), Some(0.5));
    state.snapshot.coherence = SnapshotCoherence::ExcessiveSkew;
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Degraded);
    assert_eq!(p.slip_lat_mps2.status, SignalStatus::Degraded);
}

#[test]
fn non_finite_dynamics_fail_with_typed_reason() {
    let state = with_dynamics(heading_turn(f32::NAN), Some(0.5));
    let p = resolve(&state, &FreshnessPolicy::default());
    assert_eq!(p.turn.rate_rps.status, SignalStatus::Failed);
    assert_eq!(
        p.integrity.dynamics,
        Some(crate::validate::GroupFault::NonFinite)
    );
}
