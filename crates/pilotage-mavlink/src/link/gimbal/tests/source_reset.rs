use super::*;
use crate::{ManagerWriteRequirement, ResetPolicy, SendFence};

mod transport;

const KEY: (u8, u8) = (191, 154);
const OWNER: (u8, u8) = (GCS_SYSTEM_ID, GCS_COMPONENT_ID);
const FLAGS: u32 = 2 | 32 | 64 | 128 | 256 | 512;

fn attitude(state: &Arc<Mutex<LinkState>>, time: u32, at: Instant) {
    let mut payload = [0_u8; 32];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
    receive(state, (1, 1), 31, 246, &payload, at);
}

fn prepare_source_reset(state: &Arc<Mutex<LinkState>>, restart: bool) {
    let now = Instant::now();
    if restart {
        attitude(state, 120_000, now - Duration::from_secs(4));
        attitude(state, 0, now - Duration::from_millis(300));
    } else {
        attitude(state, u32::MAX - 1, now);
    }
}

fn finish_source_reset(state: &Arc<Mutex<LinkState>>, restart: bool) {
    let epoch = state.lock().unwrap().source_epoch;
    attitude(state, if restart { 250 } else { 0 }, Instant::now());
    assert_eq!(state.lock().unwrap().source_epoch, epoch.wrapping_add(1));
}

fn discovered(state: &Arc<Mutex<LinkState>>, time: u32, owner: (u8, u8)) {
    let now = Instant::now();
    information(state, (1, KEY.0), KEY.1, time, FLAGS, now);
    status(state, (1, KEY.0), KEY.1, time.wrapping_add(1), owner, now);
}

fn fence(state: &Arc<Mutex<LinkState>>, requirement: ManagerWriteRequirement) -> SendFence {
    let report = state.lock().unwrap().gimbals.managers[&KEY];
    SendFence::default().for_manager(
        state.clone(),
        KEY,
        report.epoch,
        report.capability_revision,
        requirement,
    )
}

#[test]
fn source_reset_requires_new_reports_without_reusing_payload_timestamps() {
    for restart in [false, true] {
        let state = Arc::new(Mutex::new(LinkState {
            reset_policy: ResetPolicy::SimulatorHeuristic,
            ..LinkState::default()
        }));
        prepare_source_reset(&state, restart);
        discovered(&state, 100, OWNER);
        feedback(&state, (1, KEY.1), 0, (0, 0), 102, Instant::now());
        let mut ack = [0; 10];
        ack[..2].copy_from_slice(&1001_u16.to_le_bytes());
        ack[2] = 2;
        ack[8] = OWNER.0;
        ack[9] = OWNER.1;
        receive(&state, (1, KEY.0), 77, 143, &ack, Instant::now());
        assert_eq!(state.lock().unwrap().gimbals.configure_acks.len(), 1);
        let before = state.lock().unwrap().gimbals.managers[&KEY];
        finish_source_reset(&state, restart);
        discovered(&state, 100, OWNER);
        {
            let state = state.lock().unwrap();
            let report = state.gimbals.managers[&KEY];
            assert_eq!(report.epoch, before.epoch.wrapping_add(1));
            assert_eq!(
                report.capability_revision,
                before.capability_revision.wrapping_add(1)
            );
            assert!(report.information.is_none());
            assert!(report.status.is_none());
            assert!(state.gimbals.configure_acks.is_empty());
            assert!(state.gimbals.devices[&(KEY.1, 0)].sample.is_none());
        }
        let now = Instant::now();
        information(&state, (1, KEY.0), KEY.1, 103, FLAGS, now);
        assert!(
            !fence(&state, ManagerWriteRequirement::Primary)
                .with_valid_write(|| Ok(true))
                .unwrap()
        );
        status(&state, (1, KEY.0), KEY.1, 104, OWNER, now);
        feedback(&state, (1, KEY.1), 0, (0, 0), 102, now);
        assert!(
            state.lock().unwrap().gimbals.devices[&(KEY.1, 0)]
                .sample
                .is_none()
        );
        feedback(&state, (1, KEY.1), 0, (0, 0), 105, now);
        assert!(
            state
                .lock()
                .unwrap()
                .gimbals
                .device_for_manager_cached(KEY.0, KEY.1)
                .is_some()
        );
    }
}

#[test]
fn old_claim_motion_stop_and_release_stay_fenced_after_rediscovery() {
    for requirement in [
        ManagerWriteRequirement::Unclaimed,
        ManagerWriteRequirement::Primary,
        ManagerWriteRequirement::Stop,
        ManagerWriteRequirement::Release,
    ] {
        let state = Arc::new(Mutex::new(LinkState::default()));
        prepare_source_reset(&state, false);
        let owner = if matches!(requirement, ManagerWriteRequirement::Unclaimed) {
            (0, 0)
        } else {
            OWNER
        };
        discovered(&state, 100, owner);
        let old = fence(&state, requirement);
        assert!(old.with_valid_write(|| Ok(true)).unwrap());
        finish_source_reset(&state, false);
        assert!(
            !old.with_valid_write(|| panic!("old attachment cannot write"))
                .unwrap()
        );
        discovered(&state, 103, owner);
        assert!(
            !old.with_valid_write(|| panic!("rediscovery cannot renew old work"))
                .unwrap()
        );
        assert!(
            fence(&state, requirement)
                .with_valid_write(|| Ok(true))
                .unwrap()
        );
        status(&state, (1, KEY.0), KEY.1, 105, (42, 190), Instant::now());
        assert!(
            !fence(&state, requirement)
                .with_valid_write(|| panic!("another owner cannot authorize writing"))
                .unwrap()
        );
    }
}

#[test]
fn manager_clock_wrap_cannot_reuse_a_source_reset_discovery_epoch() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    prepare_source_reset(&state, false);
    discovered(&state, u32::MAX - 4, OWNER);
    finish_source_reset(&state, false);
    let reset_epoch = state.lock().unwrap().gimbals.managers[&KEY].epoch;
    discovered(&state, u32::MAX - 2, OWNER);
    let before_wrap = fence(&state, ManagerWriteRequirement::Primary);
    status(&state, (1, KEY.0), KEY.1, 0, OWNER, Instant::now());
    assert_eq!(
        state.lock().unwrap().gimbals.managers[&KEY].epoch,
        reset_epoch.wrapping_add(1)
    );
    discovered(&state, 1, OWNER);
    assert!(
        !before_wrap
            .with_valid_write(|| panic!("clock wrap cannot renew old work"))
            .unwrap()
    );
    assert!(
        fence(&state, ManagerWriteRequirement::Primary)
            .with_valid_write(|| Ok(true))
            .unwrap()
    );
}
