#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use crate::mavlink::AviateMessage;

use super::{LatestAviate, apply_messages};

fn attitude(sysid: u8, qw: f32) -> (u8, AviateMessage) {
    attitude_at(sysid, 1, qw)
}

fn attitude_at(sysid: u8, time_boot_ms: u32, qw: f32) -> (u8, AviateMessage) {
    (
        sysid,
        AviateMessage::AttitudeQuaternion {
            time_boot_ms,
            quat_wxyz: [qw, 0.0, 0.0, 0.0],
            rates_rps: [0.0; 3],
        },
    )
}

fn kinematics_at(sysid: u8, time_boot_ms: u32, north: f32) -> (u8, AviateMessage) {
    (
        sysid,
        AviateMessage::LocalPositionNed {
            time_boot_ms,
            pos_ned_m: [north, 0.0, 0.0],
            vel_ned_mps: [0.0; 3],
        },
    )
}

#[test]
fn locks_onto_the_first_vehicle_and_ignores_the_rest() {
    let state = Arc::new(Mutex::new(LatestAviate::default()));
    // A GCS peer heartbeat must not lock the link.
    apply_messages(
        &state,
        &[(255, AviateMessage::Heartbeat { armed: false })],
        0,
        0,
    );
    assert!(state.lock().expect("lock").locked_sysid.is_none());

    // First estimate locks; a second vehicle's estimate is ignored.
    apply_messages(&state, &[attitude(1, 0.5), attitude(2, 0.9)], 0, 0);
    let latest = state.lock().expect("lock");
    assert_eq!(latest.locked_sysid, Some(1));
    let att = latest.attitude.expect("attitude cached");
    assert_eq!(att.quat_wxyz[0], 0.5, "vehicle 2's estimate must not win");
    assert_eq!(att.stamp.source_id, 1);
    assert_eq!(att.stamp.source_epoch, 1);
}

#[test]
fn duplicate_and_reordered_group_updates_do_not_replace_the_cache() {
    let state = Arc::new(Mutex::new(LatestAviate::default()));
    apply_messages(&state, &[attitude_at(1, 100, 0.5)], 0, 0);
    apply_messages(&state, &[attitude_at(1, 100, 0.7)], 0, 0);
    apply_messages(&state, &[attitude_at(1, 99, 0.9)], 0, 0);

    let latest = state.lock().expect("lock");
    let att = latest.attitude.expect("attitude cached");
    assert_eq!(att.quat_wxyz[0], 0.5);
    assert_eq!(att.stamp.sequence, 0);
    assert_eq!(latest.duplicate_measurements, 1);
    assert_eq!(latest.reordered_measurements, 1);
}

#[test]
fn advancing_groups_keep_independent_sequences() {
    let state = Arc::new(Mutex::new(LatestAviate::default()));
    apply_messages(
        &state,
        &[attitude_at(1, 100, 0.5), kinematics_at(1, 90, 1.0)],
        0,
        0,
    );
    apply_messages(
        &state,
        &[attitude_at(1, 110, 0.6), kinematics_at(1, 100, 2.0)],
        0,
        0,
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.attitude.expect("attitude").stamp.sequence, 1);
    assert_eq!(latest.kinematics.expect("kinematics").stamp.sequence, 1);
}

#[test]
fn reboot_requires_confirming_low_clock_progress() {
    let state = Arc::new(Mutex::new(LatestAviate::default()));
    apply_messages(
        &state,
        &[attitude_at(1, 60_000, 0.5), kinematics_at(1, 60_000, 1.0)],
        0,
        0,
    );
    apply_messages(&state, &[attitude_at(1, 100, 0.7)], 0, 0);
    {
        let latest = state.lock().expect("lock");
        assert_eq!(latest.source_epoch, 1);
        assert_eq!(latest.attitude.expect("old attitude").time_boot_ms, 60_000);
    }

    apply_messages(&state, &[kinematics_at(1, 100, 2.0)], 0, 0);
    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 2);
    assert!(latest.attitude.is_none());
    assert_eq!(
        latest
            .kinematics
            .expect("new epoch sample")
            .stamp
            .source_epoch,
        2
    );
    assert_eq!(latest.source_resets, 1);
}

#[test]
fn current_epoch_progress_cancels_an_unconfirmed_reset() {
    let state = Arc::new(Mutex::new(LatestAviate::default()));
    apply_messages(&state, &[attitude_at(1, 60_000, 0.5)], 0, 0);
    apply_messages(&state, &[attitude_at(1, 100, 0.6)], 0, 0);
    apply_messages(&state, &[attitude_at(1, 60_010, 0.7)], 0, 0);
    apply_messages(&state, &[kinematics_at(1, 100, 2.0)], 0, 0);

    let latest = state.lock().expect("lock");
    assert_eq!(latest.source_epoch, 1);
    assert_eq!(
        latest.attitude.expect("current attitude").time_boot_ms,
        60_010
    );
    assert!(latest.kinematics.is_none());
}

#[test]
fn boot_clock_wrap_starts_an_explicit_epoch() {
    let state = Arc::new(Mutex::new(LatestAviate::default()));
    apply_messages(&state, &[attitude_at(1, u32::MAX - 5, 0.5)], 0, 0);
    apply_messages(&state, &[attitude_at(1, 3, 0.6)], 0, 0);

    let latest = state.lock().expect("lock");
    let att = latest.attitude.expect("wrapped sample");
    assert_eq!(latest.source_epoch, 2);
    assert_eq!(att.stamp.source_epoch, 2);
    assert_eq!(att.stamp.sequence, 0);
}
