#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::codec::{FcMessage, FrameSource};
use crate::link::{LinkState, ResetPolicy, apply_messages_at};

use super::{EstimatorAuthorization, QUALITY_DEGRADED, QUALITY_GOOD, QUALITY_UNUSABLE};

const SOURCE: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

fn state() -> Arc<Mutex<LinkState>> {
    Arc::new(Mutex::new(LinkState::default()))
}

fn simulator_state() -> Arc<Mutex<LinkState>> {
    Arc::new(Mutex::new(LinkState {
        reset_policy: ResetPolicy::SimulatorHeuristic,
        maximum_inter_group_skew_ms: 300,
        ..LinkState::default()
    }))
}

fn private_status(time_usec: u64, valid_flags: u8, quality: u8) -> FcMessage {
    FcMessage::AviateEstimatorStatus {
        time_usec,
        valid_flags,
        quality,
    }
}

fn attitude(time_boot_ms: u32) -> FcMessage {
    FcMessage::AttitudeQuaternion {
        time_boot_ms,
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        rates_rps: [0.0; 3],
    }
}

fn kinematics(time_boot_ms: u32) -> FcMessage {
    FcMessage::LocalPositionNed {
        time_boot_ms,
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
    }
}

fn apply(state: &Arc<Mutex<LinkState>>, messages: &[FcMessage]) {
    apply_at(state, messages, Instant::now());
}

fn apply_at(state: &Arc<Mutex<LinkState>>, messages: &[FcMessage], now: Instant) {
    let messages = messages
        .iter()
        .copied()
        .map(|message| (SOURCE, message))
        .collect::<Vec<_>>();
    apply_messages_at(state, &messages, 0, 0, now);
}

#[test]
fn maps_fc_quality_and_masks_unknown_validity_bits() {
    assert_eq!(
        EstimatorAuthorization::from_fc(0xff, 2),
        EstimatorAuthorization {
            valid_flags: 0x0f,
            quality: QUALITY_GOOD,
        }
    );
    assert_eq!(
        EstimatorAuthorization::from_fc(0x03, 1).quality,
        QUALITY_DEGRADED
    );
    assert_eq!(
        EstimatorAuthorization::from_fc(0x0f, 0).quality,
        QUALITY_UNUSABLE
    );
    assert_eq!(
        EstimatorAuthorization::from_fc(0x0f, 7),
        EstimatorAuthorization::fail_closed()
    );
}

#[test]
fn missing_mismatched_and_standard_status_fail_closed() {
    let state = state();
    apply(&state, &[attitude(100)]);
    apply(&state, &[private_status(200_000, 0x0f, 2), kinematics(201)]);
    apply(
        &state,
        &[
            FcMessage::EstimatorStatus {
                time_usec: 202_000,
                flags: u16::MAX,
            },
            attitude(202),
        ],
    );

    let latest = state.lock().expect("lock");
    let attitude = latest.attitude.expect("attitude");
    let kinematics = latest.kinematics.expect("kinematics");
    assert_eq!(
        (attitude.valid_flags, attitude.quality),
        (0, QUALITY_UNUSABLE)
    );
    assert_eq!(
        (kinematics.valid_flags, kinematics.quality),
        (0, QUALITY_UNUSABLE)
    );
}

#[test]
fn status_arriving_after_numeric_does_not_regrant_it() {
    let state = state();
    apply(&state, &[attitude(100)]);
    apply(&state, &[private_status(100_000, 0x0f, 2)]);

    let latest = state.lock().expect("lock");
    let attitude = latest.attitude.expect("attitude");
    assert_eq!(
        (attitude.valid_flags, attitude.quality),
        (0, QUALITY_UNUSABLE)
    );
    assert_eq!(
        latest.estimator_status_stamp().map(|stamp| stamp.sequence),
        Some(0)
    );
}

#[test]
fn later_status_can_revoke_but_not_restore_cached_numeric() {
    let state = state();
    apply(
        &state,
        &[
            private_status(100_000, 0x0f, 2),
            attitude(100),
            kinematics(100),
        ],
    );
    apply(&state, &[private_status(200_000, 0, 0)]);
    apply(&state, &[private_status(300_000, 0x0f, 2)]);

    let latest = state.lock().expect("lock");
    let attitude = latest.attitude.expect("attitude");
    let kinematics = latest.kinematics.expect("kinematics");
    assert_eq!(
        (attitude.valid_flags, attitude.quality),
        (0, QUALITY_UNUSABLE)
    );
    assert_eq!(
        (kinematics.valid_flags, kinematics.quality),
        (0, QUALITY_UNUSABLE)
    );
    assert_eq!(
        latest.estimator_status_stamp().map(|stamp| stamp.sequence),
        Some(2)
    );
}

#[test]
fn new_exact_status_and_numeric_pair_restores_authorization() {
    let state = state();
    apply(
        &state,
        &[
            private_status(100_000, 0, 0),
            attitude(100),
            kinematics(100),
            private_status(200_000, 0x0f, 2),
            attitude(200),
            kinematics(200),
        ],
    );

    let latest = state.lock().expect("lock");
    let attitude = latest.attitude.expect("attitude");
    let kinematics = latest.kinematics.expect("kinematics");
    assert_eq!(
        (attitude.valid_flags, attitude.quality),
        (0x0f, QUALITY_GOOD)
    );
    assert_eq!(
        (kinematics.valid_flags, kinematics.quality),
        (0x0f, QUALITY_GOOD)
    );
}

#[test]
fn status_clock_wrap_starts_epoch_and_clears_numeric_groups() {
    let state = state();
    let high_ms = u32::MAX - 5;
    apply(
        &state,
        &[
            private_status(u64::from(high_ms) * 1_000, 0x0f, 2),
            attitude(high_ms),
            kinematics(high_ms),
        ],
    );
    apply(&state, &[private_status(3_000, 0x0f, 2)]);

    {
        let latest = state.lock().expect("lock");
        assert_eq!(latest.source_epoch, 2);
        assert_eq!(
            latest
                .estimator_status_stamp()
                .map(|stamp| (stamp.source_epoch, stamp.sequence)),
            Some((2, 0))
        );
        assert!(latest.attitude.is_none());
        assert!(latest.kinematics.is_none());
    }

    apply(
        &state,
        &[private_status(4_000, 0x0f, 2), attitude(4), kinematics(4)],
    );
    let latest = state.lock().expect("lock");
    assert_eq!(
        latest.estimator_status_stamp().map(|stamp| stamp.sequence),
        Some(1)
    );
    assert_eq!(latest.attitude.expect("attitude").quality, QUALITY_GOOD);
    assert_eq!(latest.kinematics.expect("kinematics").quality, QUALITY_GOOD);
}

#[test]
fn duplicate_and_reordered_status_do_not_advance_its_sequence() {
    let state = state();
    apply(&state, &[private_status(100_000, 0x0f, 2)]);
    apply(&state, &[private_status(100_000, 0, 0)]);
    apply(&state, &[private_status(99_000, 0, 0)]);

    let latest = state.lock().expect("lock");
    assert_eq!(
        latest.estimator_status_stamp().map(|stamp| stamp.sequence),
        Some(0)
    );
    assert_eq!(latest.duplicate_measurements, 1);
    assert_eq!(latest.reordered_measurements, 1);
}

#[test]
fn active_low_status_clock_is_rejected() {
    let state = simulator_state();
    let start = Instant::now();
    apply_at(&state, &[private_status(60_000_000, 0x0f, 2)], start);
    apply_at(
        &state,
        &[private_status(100_000, 0, 0)],
        start + Duration::from_millis(100),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 1);
    assert_eq!(
        latest.estimator_status.expect("status").time_boot_ms,
        60_000
    );
    assert_eq!(latest.reordered_measurements, 1);
}

#[test]
fn status_only_reset_requires_silence_source_progress_and_receive_dwell() {
    let state = simulator_state();
    let start = Instant::now();
    apply_at(&state, &[private_status(60_000_000, 0x0f, 2)], start);
    apply_at(
        &state,
        &[private_status(100_000, 0x0f, 2)],
        start + Duration::from_secs(4),
    );
    apply_at(
        &state,
        &[private_status(200_000, 0x0f, 2)],
        start + Duration::from_millis(4_100),
    );
    assert_eq!(state.lock().expect("lock").source_epoch, 1);
    apply_at(
        &state,
        &[private_status(400_000, 0x0f, 2)],
        start + Duration::from_millis(4_400),
    );

    let latest = state.lock().expect("lock");
    let stamp = latest.estimator_status_stamp().expect("new status");
    assert_eq!((latest.source_epoch, stamp.source_epoch), (2, 2));
    assert_eq!(stamp.sequence, 0);
    assert_eq!(latest.source_resets, 1);
}

#[test]
fn a_different_measurement_group_cannot_confirm_status_reset() {
    let state = simulator_state();
    let start = Instant::now();
    apply_at(&state, &[private_status(60_000_000, 0x0f, 2)], start);
    apply_at(
        &state,
        &[private_status(100_000, 0x0f, 2)],
        start + Duration::from_secs(4),
    );
    apply_at(
        &state,
        &[attitude(400)],
        start + Duration::from_millis(4_400),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 1);
    assert!(latest.attitude.is_none());
    assert_eq!(
        latest.estimator_status.expect("old status").time_boot_ms,
        60_000
    );
    assert_eq!(latest.source_resets, 0);
}

#[test]
fn status_sequence_is_independent_gap_insensitive_and_wrapping() {
    let state = state();
    apply(&state, &[private_status(100_000, 0x0f, 2), attitude(100)]);
    apply(&state, &[private_status(5_000_000, 0x0f, 2)]);
    {
        let mut latest = state.lock().expect("lock");
        assert_eq!(latest.attitude.expect("attitude").stamp.sequence, 0);
        assert_eq!(
            latest.estimator_status_stamp().map(|stamp| stamp.sequence),
            Some(1)
        );
        latest
            .estimator_status
            .as_mut()
            .expect("status")
            .stamp
            .sequence = u32::MAX;
    }
    apply(&state, &[private_status(6_000_000, 0x0f, 2)]);

    assert_eq!(
        state
            .lock()
            .expect("lock")
            .estimator_status_stamp()
            .map(|stamp| stamp.sequence),
        Some(0)
    );
}

#[test]
fn non_millisecond_status_propagates_fail_closed_without_authorizing() {
    let state = state();
    apply(
        &state,
        &[
            private_status(100_000, 0x0f, 2),
            attitude(100),
            kinematics(100),
        ],
    );
    apply(&state, &[private_status(200_001, 0x0f, 2)]);

    let latest = state.lock().expect("lock");
    let status = latest.estimator_status.expect("status");
    assert_eq!(status.stamp.acquired_at_ns, 200_001_000);
    assert_eq!(status.authorization, EstimatorAuthorization::fail_closed());
    assert_eq!(latest.attitude.expect("attitude").quality, QUALITY_UNUSABLE);
    assert_eq!(
        latest.kinematics.expect("kinematics").quality,
        QUALITY_UNUSABLE
    );
    assert_eq!(latest.invalid_estimator_statuses, 1);
}

#[test]
fn same_millisecond_malformed_status_revokes_the_cached_pair() {
    let state = state();
    apply(
        &state,
        &[
            private_status(100_000, 0x0f, 2),
            attitude(100),
            kinematics(100),
        ],
    );
    apply(&state, &[private_status(100_001, 0x0f, 2)]);

    let latest = state.lock().expect("lock");
    assert_eq!(
        latest
            .estimator_status
            .expect("last valid status")
            .time_usec,
        100_000
    );
    assert_eq!(latest.attitude.expect("attitude").quality, QUALITY_UNUSABLE);
    assert_eq!(
        latest.kinematics.expect("kinematics").quality,
        QUALITY_UNUSABLE
    );
    assert_eq!(latest.invalid_estimator_statuses, 1);
}

#[test]
fn same_timestamp_unknown_quality_revokes_the_cached_pair() {
    let state = state();
    apply(
        &state,
        &[
            private_status(100_000, 0x0f, 2),
            attitude(100),
            kinematics(100),
        ],
    );
    apply(&state, &[private_status(100_000, 0x0f, u8::MAX)]);

    let latest = state.lock().expect("lock");
    assert_eq!(latest.attitude.expect("attitude").quality, QUALITY_UNUSABLE);
    assert_eq!(
        latest.kinematics.expect("kinematics").quality,
        QUALITY_UNUSABLE
    );
    assert_eq!(latest.invalid_estimator_statuses, 1);
}
