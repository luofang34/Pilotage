#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::SourceIncarnation;

use crate::mavlink::{AviateMessage, FrameSource};

use super::{LatestAviate, ResetPolicy, apply_messages_at};

const SELECTED: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

fn state(policy: ResetPolicy) -> Arc<Mutex<LatestAviate>> {
    let maximum_inter_group_skew_ms = if policy == ResetPolicy::SimulatorHeuristic {
        300
    } else {
        0
    };
    state_with_skew(policy, maximum_inter_group_skew_ms)
}

fn state_with_skew(
    policy: ResetPolicy,
    maximum_inter_group_skew_ms: u32,
) -> Arc<Mutex<LatestAviate>> {
    Arc::new(Mutex::new(LatestAviate {
        reset_policy: policy,
        maximum_inter_group_skew_ms,
        source_incarnation: SourceIncarnation::new([0xA5; 16]),
        ..LatestAviate::default()
    }))
}

fn attitude_at(time_boot_ms: u32, qw: f32) -> (FrameSource, AviateMessage) {
    (
        SELECTED,
        AviateMessage::AttitudeQuaternion {
            time_boot_ms,
            quat_wxyz: [qw, 0.0, 0.0, 0.0],
            rates_rps: [0.0; 3],
        },
    )
}

fn kinematics_at(time_boot_ms: u32, north: f32) -> (FrameSource, AviateMessage) {
    (
        SELECTED,
        AviateMessage::LocalPositionNed {
            time_boot_ms,
            pos_ned_m: [north, 0.0, 0.0],
            vel_ned_mps: [0.0; 3],
        },
    )
}

fn apply_at(
    state: &Arc<Mutex<LatestAviate>>,
    messages: &[(FrameSource, AviateMessage)],
    now: Instant,
) {
    apply_messages_at(state, messages, 0, 0, now);
}

#[test]
fn accepts_only_the_configured_system_and_component() {
    let state = state(ResetPolicy::Conservative);
    let mut wrong_system = attitude_at(1, 0.8);
    wrong_system.0.system_id = 2;
    let mut wrong_component = attitude_at(2, 0.9);
    wrong_component.0.component_id = 42;
    apply_at(
        &state,
        &[wrong_system, wrong_component, attitude_at(3, 0.5)],
        Instant::now(),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(
        latest.attitude.expect("selected attitude").quat_wxyz[0],
        0.5
    );
    assert_eq!(latest.wrong_sources, 2);
    assert_eq!(latest.attitude.expect("attitude").stamp.source_id, 1);
    assert_eq!(
        latest.attitude.expect("attitude").stamp.source_incarnation,
        SourceIncarnation::new([0xA5; 16])
    );
}

#[test]
fn duplicate_and_reordered_group_updates_do_not_replace_the_cache() {
    let state = state(ResetPolicy::Conservative);
    let now = Instant::now();
    apply_at(&state, &[attitude_at(100, 0.5)], now);
    apply_at(&state, &[attitude_at(100, 0.7)], now);
    apply_at(&state, &[attitude_at(99, 0.9)], now);

    let latest = state.lock().expect("lock");
    let att = latest.attitude.expect("attitude cached");
    assert_eq!(att.quat_wxyz[0], 0.5);
    assert_eq!(att.stamp.sequence, 0);
    assert_eq!(latest.duplicate_measurements, 1);
    assert_eq!(latest.reordered_measurements, 1);
}

#[test]
fn advancing_groups_keep_independent_sequences() {
    let state = state_with_skew(ResetPolicy::Conservative, 20);
    let now = Instant::now();
    apply_at(
        &state,
        &[attitude_at(100, 0.5), kinematics_at(90, 1.0)],
        now,
    );
    apply_at(
        &state,
        &[attitude_at(110, 0.6), kinematics_at(100, 2.0)],
        now + Duration::from_millis(10),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.attitude.expect("attitude").stamp.sequence, 1);
    assert_eq!(latest.kinematics.expect("kinematics").stamp.sequence, 1);
    assert_eq!(latest.last_source_time_ms, Some(110));
}

#[test]
fn active_stream_low_clock_replay_is_rejected() {
    let state = state(ResetPolicy::SimulatorHeuristic);
    let start = Instant::now();
    apply_at(&state, &[attitude_at(60_000, 0.5)], start);
    apply_at(
        &state,
        &[attitude_at(100, 0.9)],
        start + Duration::from_millis(100),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 1);
    assert_eq!(latest.attitude.expect("current").time_boot_ms, 60_000);
    assert!(latest.pending_reset.is_none());
    assert_eq!(latest.reordered_measurements, 1);
}

#[test]
fn delayed_single_replay_stays_quarantined() {
    let state = state(ResetPolicy::SimulatorHeuristic);
    let start = Instant::now();
    apply_at(&state, &[attitude_at(60_000, 0.5)], start);
    apply_at(
        &state,
        &[attitude_at(100, 0.9)],
        start + Duration::from_secs(4),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 1);
    assert_eq!(latest.attitude.expect("old attitude").time_boot_ms, 60_000);
    assert!(latest.pending_reset.is_some());
    assert_eq!(latest.suspected_resets, 1);
}

#[test]
fn same_datagram_cross_group_replay_cannot_confirm_reset() {
    let state = state(ResetPolicy::SimulatorHeuristic);
    let start = Instant::now();
    apply_at(
        &state,
        &[attitude_at(60_000, 0.5), kinematics_at(60_000, 1.0)],
        start,
    );
    apply_at(
        &state,
        &[attitude_at(100, 0.9), kinematics_at(100, 9.0)],
        start + Duration::from_secs(4),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 1);
    assert_eq!(latest.attitude.expect("old attitude").time_boot_ms, 60_000);
    assert_eq!(
        latest.kinematics.expect("old kinematics").time_boot_ms,
        60_000
    );
    assert_eq!(latest.source_resets, 0);
}

fn confirm_attitude_reset(state: &Arc<Mutex<LatestAviate>>, start: Instant, first_low_ms: u32) {
    apply_at(
        state,
        &[attitude_at(first_low_ms, 0.6)],
        start + Duration::from_secs(4),
    );
    apply_at(
        state,
        &[attitude_at(first_low_ms.wrapping_add(100), 0.7)],
        start + Duration::from_millis(4_100),
    );
    apply_at(
        state,
        &[attitude_at(first_low_ms.wrapping_add(300), 0.8)],
        start + Duration::from_millis(4_400),
    );
}

#[test]
fn simulator_reset_requires_same_group_source_and_receive_dwell() {
    let state = state(ResetPolicy::SimulatorHeuristic);
    let start = Instant::now();
    apply_at(&state, &[attitude_at(60_000, 0.5)], start);
    confirm_attitude_reset(&state, start, 100);

    let latest = state.lock().expect("lock");
    let attitude = latest.attitude.expect("new epoch sample");
    assert_eq!(latest.source_epoch, 2);
    assert_eq!(attitude.time_boot_ms, 400);
    assert_eq!(attitude.stamp.source_epoch, 2);
    assert_eq!(attitude.stamp.sequence, 0);
    assert_eq!(latest.source_resets, 1);
}

#[test]
fn short_prior_boot_can_transition_only_after_silence_and_dwell() {
    let state = state(ResetPolicy::SimulatorHeuristic);
    let start = Instant::now();
    apply_at(&state, &[attitude_at(1_000, 0.5)], start);
    confirm_attitude_reset(&state, start, 10);
    assert_eq!(state.lock().expect("lock").source_epoch, 2);
}

#[test]
fn conservative_policy_never_infers_replayable_reset() {
    let state = state(ResetPolicy::Conservative);
    let start = Instant::now();
    apply_at(&state, &[attitude_at(60_000, 0.5)], start);
    confirm_attitude_reset(&state, start, 100);

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 1);
    assert_eq!(latest.attitude.expect("old attitude").time_boot_ms, 60_000);
}

#[test]
fn boot_clock_wrap_starts_an_explicit_epoch_without_reboot_heuristic() {
    let state = state(ResetPolicy::Conservative);
    let start = Instant::now();
    apply_at(&state, &[attitude_at(u32::MAX - 5, 0.5)], start);
    apply_at(
        &state,
        &[attitude_at(3, 0.8)],
        start + Duration::from_millis(10),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 2);
    assert_eq!(latest.attitude.expect("wrapped sample").time_boot_ms, 3);
}

#[test]
fn absent_group_quarantines_low_clock_replay_below_epoch_high_water() {
    let state = state(ResetPolicy::SimulatorHeuristic);
    let start = Instant::now();
    apply_at(&state, &[attitude_at(60_000, 0.5)], start);
    apply_at(
        &state,
        &[kinematics_at(100, 9.0)],
        start + Duration::from_secs(4),
    );

    let latest = state.lock().expect("lock");
    assert!(latest.kinematics.is_none());
    assert_eq!(latest.last_source_time_ms, Some(60_000));
    assert_eq!(latest.suspected_resets, 1);
    assert!(latest.pending_reset.is_some());
}

#[test]
fn established_group_cannot_advance_far_behind_epoch_high_water() {
    let state = state(ResetPolicy::SimulatorHeuristic);
    let start = Instant::now();
    apply_at(&state, &[kinematics_at(1_000, 1.0)], start);
    apply_at(
        &state,
        &[attitude_at(10_000, 0.5)],
        start + Duration::from_millis(1),
    );
    apply_at(
        &state,
        &[kinematics_at(1_100, 9.0)],
        start + Duration::from_millis(2),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.last_source_time_ms, Some(10_000));
    assert_eq!(latest.kinematics.expect("current").time_boot_ms, 1_000);
    assert_eq!(latest.reordered_measurements, 1);
}

#[test]
fn current_epoch_progress_cancels_an_unconfirmed_reset() {
    let state = state(ResetPolicy::SimulatorHeuristic);
    let start = Instant::now();
    apply_at(&state, &[attitude_at(60_000, 0.5)], start);
    apply_at(
        &state,
        &[attitude_at(100, 0.6)],
        start + Duration::from_secs(4),
    );
    apply_at(
        &state,
        &[attitude_at(60_010, 0.7)],
        start + Duration::from_millis(4_010),
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 1);
    assert_eq!(latest.attitude.expect("current").time_boot_ms, 60_010);
    assert!(latest.pending_reset.is_none());
}
