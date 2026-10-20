use super::*;

#[test]
fn revocation_serializes_with_an_in_progress_nonblocking_write() {
    let fence = SendFence::default();
    let writer_fence = fence.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        writer_fence
            .with_valid_write(|| {
                entered_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                Ok(true)
            })
            .unwrap()
    });
    entered_rx.recv().unwrap();
    assert!(fence.valid.try_lock().is_err());
    finish_tx.send(()).unwrap();
    assert!(writer.join().unwrap());
    fence.invalidate();
    assert!(
        !fence
            .with_valid_write(|| panic!("revoked operation must not run"))
            .unwrap()
    );
}

fn status(state: &Arc<Mutex<super::super::LinkState>>, time: u32, device: u8, owner: (u8, u8)) {
    let mut payload = [0_u8; 13];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[8] = device;
    payload[9] = owner.0;
    payload[10] = owner.1;
    let mut bytes = [0_u8; 25];
    let length = crate::codec::encode_frame_v2_from(1, 281, &payload, 48, 1, 1, &mut bytes);
    let mut messages = Vec::new();
    let stats = crate::parse_datagram(&bytes[..length], &mut messages);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(messages.len(), 1);
    super::super::apply_messages_at(state, &messages, 0, 0, std::time::Instant::now());
}

fn discovered_manager(
    state: &Arc<Mutex<super::super::LinkState>>,
    component: u8,
    device: u8,
) -> (u64, u64) {
    let mut payload = [0_u8; 33];
    payload[4..8].copy_from_slice(&0x138_u32.to_le_bytes());
    payload[32] = device;
    let mut bytes = [0_u8; 45];
    let length = crate::codec::encode_frame_v2_from(1, 280, &payload, 70, 1, component, &mut bytes);
    let mut messages = Vec::new();
    let stats = crate::parse_datagram(&bytes[..length], &mut messages);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(messages.len(), 1);
    super::super::apply_messages_at(state, &messages, 0, 0, std::time::Instant::now());
    let state = state.lock().unwrap();
    let report = state.gimbals.managers.get(&(component, device)).unwrap();
    (report.epoch, report.capability_revision)
}

#[test]
fn received_competing_owner_fences_write_before_adapter_poll() {
    let state = Arc::new(Mutex::new(super::super::LinkState::default()));
    let (epoch, revision) = discovered_manager(&state, 1, 2);
    let fence = SendFence::default().for_manager(
        state.clone(),
        (1, 2),
        epoch,
        revision,
        ManagerWriteRequirement::Primary,
    );
    status(
        &state,
        1,
        2,
        (crate::codec::GCS_SYSTEM_ID, crate::codec::GCS_COMPONENT_ID),
    );
    assert!(fence.with_valid_write(|| Ok(true)).unwrap());
    status(&state, 2, 2, (42, 190));
    assert!(
        !fence
            .with_valid_write(|| panic!("competing manager owner must block queued writes"))
            .unwrap()
    );
}

#[test]
fn claims_require_fresh_free_owner_and_selected_device() {
    let state = Arc::new(Mutex::new(super::super::LinkState::default()));
    let (epoch, revision) = discovered_manager(&state, 1, 2);
    let fence = SendFence::default().for_manager(
        state.clone(),
        (1, 2),
        epoch,
        revision,
        ManagerWriteRequirement::Unclaimed,
    );
    status(&state, 1, 2, (0, 0));
    assert!(fence.with_valid_write(|| Ok(true)).unwrap());
    status(&state, 2, 2, (42, 190));
    assert!(
        !fence
            .with_valid_write(|| panic!("claim must not steal ownership"))
            .unwrap()
    );
    status(&state, 3, 3, (0, 0));
    assert!(
        !fence
            .with_valid_write(|| panic!("different device must not authorize claim"))
            .unwrap()
    );
}

#[test]
fn self_release_accepts_unowned_status_but_never_releases_a_new_owner() {
    let state = Arc::new(Mutex::new(super::super::LinkState::default()));
    let (epoch, revision) = discovered_manager(&state, 1, 2);
    let fence = SendFence::default().for_manager(
        state.clone(),
        (1, 2),
        epoch,
        revision,
        ManagerWriteRequirement::Release,
    );
    status(&state, 1, 2, (0, 0));
    assert!(fence.with_valid_write(|| Ok(true)).unwrap());
    status(&state, 2, 2, (42, 190));
    assert!(
        !fence
            .with_valid_write(|| panic!("new owner must not be released"))
            .unwrap()
    );
    status(
        &state,
        3,
        2,
        (crate::codec::GCS_SYSTEM_ID, crate::codec::GCS_COMPONENT_ID),
    );
    assert!(fence.with_valid_write(|| Ok(true)).unwrap());
}

fn camera_report(
    state: &Arc<Mutex<super::super::LinkState>>,
    received: std::time::Instant,
) -> (u64, u64) {
    let mut payload = [0_u8; 237];
    payload[..4].copy_from_slice(&100_u32.to_le_bytes());
    payload[20..24].copy_from_slice(&64_u32.to_le_bytes());
    let mut bytes = [0_u8; 260];
    let length = crate::codec::encode_frame_v2_from(1, 259, &payload, 92, 1, 100, &mut bytes);
    let mut messages = Vec::new();
    let stats = crate::parse_datagram(&bytes[..length], &mut messages);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(messages.len(), 1);
    super::super::apply_messages_at(state, &messages, 0, 0, received);
    let state = state.lock().unwrap();
    let report = state.fc_reports.cameras.get(&(100, 0)).unwrap();
    (report.epoch, report.capability_revision)
}

#[test]
fn camera_writer_requires_exact_discovery_binding_and_holds_receive_lock() {
    let state = Arc::new(Mutex::new(super::super::LinkState::default()));
    let (epoch, revision) = camera_report(&state, std::time::Instant::now());
    let fence = SendFence::default().for_camera(state.clone(), (100, 0), epoch, revision);
    assert!(
        fence
            .with_valid_write(|| {
                assert!(
                    state.try_lock().is_err(),
                    "a new raw report cannot race the final write"
                );
                Ok(true)
            })
            .unwrap()
    );
    for (key, expected_epoch, expected_revision) in [
        ((101, 0), epoch, revision),
        ((100, 0), epoch.wrapping_add(1), revision),
        ((100, 0), epoch, revision.wrapping_add(1)),
    ] {
        let invalid =
            SendFence::default().for_camera(state.clone(), key, expected_epoch, expected_revision);
        assert!(
            !invalid
                .with_valid_write(|| panic!(
                    "a different discovery contract cannot authorize writing"
                ))
                .unwrap()
        );
    }
}

#[test]
fn camera_discovery_ttl_fences_write_without_adapter_expiry_poll() {
    let state = Arc::new(Mutex::new(super::super::LinkState::default()));
    let (epoch, revision) = camera_report(
        &state,
        std::time::Instant::now() - std::time::Duration::from_secs(7),
    );
    assert!(
        state
            .lock()
            .unwrap()
            .fc_reports
            .cameras
            .get(&(100, 0))
            .unwrap()
            .available
    );
    let fence = SendFence::default().for_camera(state, (100, 0), epoch, revision);
    assert!(
        !fence
            .with_valid_write(|| panic!("raw cache availability does not extend discovery TTL"))
            .unwrap()
    );
}
