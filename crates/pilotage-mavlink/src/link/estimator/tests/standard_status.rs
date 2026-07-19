//! Authorization from the standard ESTIMATOR_STATUS (msg 230): the
//! bounded-lag grant, the degraded attitude-only case, and the Aviate
//! dialect's refusal to authorize from it.

use std::sync::{Arc, Mutex};

use crate::codec::FcMessage;
use crate::link::LinkState;

use super::{QUALITY_DEGRADED, QUALITY_GOOD, QUALITY_UNUSABLE, apply, attitude, kinematics, state};

fn standard_state() -> Arc<Mutex<LinkState>> {
    Arc::new(Mutex::new(LinkState {
        authorization_source: crate::AuthorizationSource::StandardEstimatorStatus,
        maximum_inter_group_skew_ms: 300,
        ..LinkState::default()
    }))
}

fn standard_status(time_usec: u64, flags: u16) -> FcMessage {
    FcMessage::EstimatorStatus { time_usec, flags }
}

#[test]
fn standard_status_authorizes_numerics_within_bounded_lag() {
    let state = standard_state();
    // Attitude + both velocity bits + relative-horizontal and vertical
    // position: full authorization.
    apply(&state, &[standard_status(1_000_000, 1 | 2 | 4 | 8 | 32)]);
    apply(&state, &[attitude(2_500), kinematics(2_500)]);

    let latest = state.lock().expect("lock");
    assert_eq!(
        (
            latest.attitude.expect("attitude").valid_flags,
            latest.attitude.expect("attitude").quality
        ),
        (0b1111, QUALITY_GOOD)
    );
    assert_eq!(
        (
            latest.kinematics.expect("kinematics").valid_flags,
            latest.kinematics.expect("kinematics").quality
        ),
        (0b1111, QUALITY_GOOD)
    );
}

#[test]
fn standard_status_beyond_the_lag_bound_fails_closed() {
    let state = standard_state();
    apply(&state, &[standard_status(1_000_000, 1 | 2 | 4 | 8 | 32)]);
    apply(&state, &[attitude(3_500)]);

    let latest = state.lock().expect("lock");
    assert_eq!(
        (
            latest.attitude.expect("attitude").valid_flags,
            latest.attitude.expect("attitude").quality
        ),
        (0, QUALITY_UNUSABLE)
    );
}

#[test]
fn standard_status_attitude_only_is_degraded() {
    let state = standard_state();
    apply(&state, &[standard_status(1_000_000, 1)]);
    apply(&state, &[attitude(1_200)]);

    let latest = state.lock().expect("lock");
    let att = latest.attitude.expect("attitude");
    assert_eq!((att.valid_flags, att.quality), (0b0011, QUALITY_DEGRADED));
}

#[test]
fn aviate_dialect_never_authorizes_from_the_standard_status() {
    let state = state();
    apply(&state, &[standard_status(1_000_000, 0xff)]);
    apply(&state, &[attitude(1_000)]);

    let latest = state.lock().expect("lock");
    let att = latest.attitude.expect("attitude");
    assert_eq!((att.valid_flags, att.quality), (0, QUALITY_UNUSABLE));
    assert!(latest.estimator_status.is_none());
}
