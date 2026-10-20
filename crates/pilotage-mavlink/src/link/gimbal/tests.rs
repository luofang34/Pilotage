use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::codec::{GCS_COMPONENT_ID, GCS_SYSTEM_ID, encode_frame_v2_from};

mod source_reset;

fn receive(
    state: &Arc<Mutex<LinkState>>,
    source: (u8, u8),
    id: u32,
    crc: u8,
    payload: &[u8],
    now: Instant,
) {
    let mut bytes = [0; 280];
    let length = encode_frame_v2_from(1, id, payload, crc, source.0, source.1, &mut bytes);
    let mut messages = Vec::new();
    let stats = crate::parse_datagram(&bytes[..length], &mut messages);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(messages.len(), 1);
    super::super::apply_messages_at(state, &messages, 0, 0, now);
}

fn information(
    state: &Arc<Mutex<LinkState>>,
    source: (u8, u8),
    device: u8,
    time: u32,
    flags: u32,
    now: Instant,
) {
    let mut payload = [0; 33];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[4..8].copy_from_slice(&flags.to_le_bytes());
    for (index, limit) in [-1.0_f32, 1.0, -1.5, 1.2, f32::NAN, f32::NAN]
        .into_iter()
        .enumerate()
    {
        payload[8 + index * 4..12 + index * 4].copy_from_slice(&limit.to_le_bytes());
    }
    payload[32] = device;
    receive(state, source, 280, 70, &payload, now);
}

fn status(
    state: &Arc<Mutex<LinkState>>,
    source: (u8, u8),
    device: u8,
    time: u32,
    owner: (u8, u8),
    now: Instant,
) {
    let mut payload = [0; 13];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[8] = device;
    payload[9] = owner.0;
    payload[10] = owner.1;
    receive(state, source, 281, 48, &payload, now);
}

fn feedback(
    state: &Arc<Mutex<LinkState>>,
    source: (u8, u8),
    device: u8,
    target: (u8, u8),
    time: u32,
    now: Instant,
) {
    let mut payload = [0; 49];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
    payload[20..24].copy_from_slice(&f32::NAN.to_le_bytes());
    payload[38] = target.0;
    payload[39] = target.1;
    payload[48] = device;
    receive(state, source, 285, 137, &payload, now);
}

#[test]
fn discovery_retains_flags_limits_and_multiple_payload_routes_without_fc_authorization() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    for (manager, device) in [(1, 2), (1, 3), (191, 154), (192, 155)] {
        information(&state, (1, manager), device, 100, 0x138, now);
        status(&state, (1, manager), device, 101, (0, 0), now);
    }
    for (sender, device) in [(1, 2), (1, 3), (154, 0), (155, 0)] {
        feedback(&state, (1, sender), device, (0, 0), 102, now);
    }
    let state = state.lock().unwrap();
    assert_eq!(state.gimbals.managers.len(), 4);
    for (manager, device) in [(1, 2), (1, 3), (191, 154), (192, 155)] {
        let info = state.gimbals.managers[&(manager, device)]
            .information
            .unwrap()
            .0;
        assert_eq!(info.cap_flags, 0x138);
        assert_eq!((info.pitch_min, info.pitch_max), (-1.5, 1.2));
        assert!(info.yaw_min.is_nan());
        let feedback = state
            .gimbals
            .device_for_manager_cached(manager, device)
            .unwrap();
        assert_eq!(
            (feedback.source_component, feedback.device_id),
            device_key(manager, device)
        );
        assert!(feedback.rates_rps[0].is_nan());
    }
    assert!(state.attitude.is_none());
    assert!(state.kinematics.is_none());
    assert!(state.estimator_status.is_none());
    assert!(state.last_heartbeat.is_none());
    assert!(state.last_source_time_ms.is_none());
    assert_eq!(state.source_epoch, 1);
}

#[test]
fn rejects_other_system_undiscovered_devices_and_mismatched_targets() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    information(&state, (2, 1), 2, 1, 1, now);
    information(&state, (1, 0), 2, 1, 1, now);
    information(&state, (1, 1), 0, 1, 1, now);
    information(&state, (1, 1), 2, 1, 1, now);
    information(&state, (1, 191), 154, 1, 1, now);
    for (source, device, target) in [
        ((2, 1), 2, (0, 0)),
        ((1, 2), 2, (0, 0)),
        ((1, 1), 3, (0, 0)),
        ((1, 1), 2, (42, 190)),
        ((1, 1), 2, (GCS_SYSTEM_ID, 7)),
        ((1, 154), 0, (1, 192)),
        ((1, 155), 0, (0, 0)),
        ((1, 1), 154, (0, 0)),
    ] {
        feedback(&state, source, device, target, 2, now);
    }
    assert!(state.lock().unwrap().gimbals.devices.is_empty());
    feedback(&state, (1, 154), 0, (1, 191), 3, now);
    feedback(&state, (1, 1), 2, (GCS_SYSTEM_ID, GCS_COMPONENT_ID), 3, now);
    let state = state.lock().unwrap();
    assert_eq!(state.gimbals.managers.len(), 2);
    assert_eq!(state.gimbals.devices.len(), 2);
    assert_eq!(state.wrong_sources, 3);
}

#[test]
fn duplicate_or_reordered_reports_do_not_renew_receipts_or_infer_reboot() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    information(&state, (1, 1), 2, 100, 1, now);
    status(&state, (1, 1), 2, 101, (0, 0), now);
    feedback(&state, (1, 1), 2, (0, 0), 101, now);
    let revision = state.lock().unwrap().gimbals.managers[&(1, 2)].capability_revision;
    for time in [100, 1] {
        information(&state, (1, 1), 2, time, 999, now + Duration::from_secs(1));
        status(
            &state,
            (1, 1),
            2,
            time,
            (42, 190),
            now + Duration::from_secs(1),
        );
        feedback(
            &state,
            (1, 1),
            2,
            (0, 0),
            time,
            now + Duration::from_secs(1),
        );
    }
    let state = state.lock().unwrap();
    let report = state.gimbals.managers[&(1, 2)];
    assert_eq!(report.epoch, 1);
    assert_eq!(report.capability_revision, revision);
    assert_eq!(report.information.unwrap().1, now);
    assert_eq!(report.status.unwrap().1, now);
    let feedback = state.gimbals.device_for_manager_cached(1, 2).unwrap();
    assert_eq!(feedback.received_at, now);
    assert_eq!(feedback.sequence, 1);
}

#[test]
fn boot_wrap_invalidates_binding_and_late_previous_epoch_reports_cannot_restore_it() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    information(&state, (1, 1), 2, u32::MAX - 2, 1, now);
    status(&state, (1, 1), 2, u32::MAX - 1, (0, 0), now);
    feedback(&state, (1, 1), 2, (0, 0), u32::MAX - 1, now);
    let revision = state.lock().unwrap().gimbals.managers[&(1, 2)].capability_revision;
    status(&state, (1, 1), 2, 1, (42, 190), now);
    information(&state, (1, 1), 2, u32::MAX, 1, now);
    assert!(
        state.lock().unwrap().gimbals.managers[&(1, 2)]
            .information
            .is_none()
    );
    assert!(
        state
            .lock()
            .unwrap()
            .gimbals
            .device_for_manager_cached(1, 2)
            .is_none()
    );
    information(&state, (1, 1), 2, 2, 1, now);
    feedback(&state, (1, 1), 2, (0, 0), u32::MAX, now);
    assert!(
        state
            .lock()
            .unwrap()
            .gimbals
            .device_for_manager_cached(1, 2)
            .is_none()
    );
    feedback(&state, (1, 1), 2, (0, 0), 3, now);
    status(&state, (1, 1), 2, u32::MAX, (0, 0), now);
    feedback(&state, (1, 1), 2, (0, 0), u32::MAX, now);
    let state = state.lock().unwrap();
    let report = state.gimbals.managers[&(1, 2)];
    assert_eq!(report.epoch, 2);
    assert!(report.capability_revision > revision);
    assert_eq!(report.status.unwrap().0.primary_system, 42);
    let feedback = state.gimbals.device_for_manager_cached(1, 2).unwrap();
    assert_eq!(
        (feedback.epoch, feedback.time_boot_ms, feedback.sequence),
        (2, 3, 2)
    );
}

#[test]
fn nan_limits_do_not_change_contract_and_delayed_independent_stream_remains_usable() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    information(&state, (1, 1), 2, 100, 1, now);
    let revision = state.lock().unwrap().gimbals.managers[&(1, 2)].capability_revision;
    status(&state, (1, 1), 2, 200, (0, 0), now);
    information(&state, (1, 1), 2, 150, 1, now);
    let state = state.lock().unwrap();
    let report = state.gimbals.managers[&(1, 2)];
    assert_eq!(report.capability_revision, revision);
    assert_eq!(report.information.unwrap().0.time_boot_ms, 150);
}

#[test]
fn discovery_capacity_is_bounded_and_existing_devices_keep_updating() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    for component in 1..=32 {
        information(&state, (1, component), 2, 1, 1, now);
    }
    information(&state, (1, 1), 2, 2, 5, now);
    let state = state.lock().unwrap();
    assert_eq!(state.gimbals.managers.len(), MAX_GIMBALS);
    assert_eq!(
        state.gimbals.managers[&(1, 2)]
            .information
            .unwrap()
            .0
            .cap_flags,
        5
    );
}

#[test]
fn configure_ack_belongs_only_to_its_discovered_manager_component() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    information(&state, (1, 191), 154, 1, 1, now);
    information(&state, (1, 192), 155, 1, 1, now);
    let mut payload = [0; 10];
    payload[..2].copy_from_slice(&crate::codec::MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE.to_le_bytes());
    payload[2] = 2;
    payload[8] = GCS_SYSTEM_ID;
    payload[9] = GCS_COMPONENT_ID;
    for source in [(1, 191), (2, 192), (1, 193)] {
        receive(&state, source, 77, 143, &payload, now);
    }
    payload[8] = 42;
    receive(&state, (1, 192), 77, 143, &payload, now);
    let state = state.lock().unwrap();
    assert_eq!(state.gimbals.configure_acks.len(), 1);
    assert_eq!(state.gimbals.configure_acks[&191].result, 2);
    assert!(state.last_command_ack.is_none());
    assert!(state.fc_reports.command_acks.is_empty());
}
