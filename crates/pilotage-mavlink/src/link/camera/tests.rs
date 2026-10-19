use super::*;
use crate::{FcMessage, FrameSource};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn report(time: u32, system: u8) -> (FrameSource, FcMessage) {
    (
        FrameSource {
            system_id: system,
            component_id: 100,
            frame_sequence: 0,
        },
        FcMessage::Camera(CameraMessage::Information {
            time_boot_ms: time,
            flags: 64,
            device_id: 0,
            gimbal_id: 1,
        }),
    )
}

#[test]
fn independent_camera_discovery_does_not_authorize_vehicle_estimates() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let now = Instant::now();
    crate::link::apply_messages_at(&state, &[report(100, 2)], 0, 0, now);
    assert!(state.lock().unwrap().fc_reports.cameras.is_empty());
    crate::link::apply_messages_at(&state, &[report(100, 1)], 0, 0, now);
    let latest = state.lock().unwrap();
    let camera = latest.fc_reports.cameras.get(&(100, 0)).unwrap();
    assert!(camera.available);
    assert_eq!(camera.flags, Some(64));
    assert!(latest.attitude.is_none());
    assert!(latest.kinematics.is_none());
}

#[test]
fn duplicate_discovery_cannot_refresh_expiry_and_new_reports_restore_it() {
    let mut state = LinkState::default();
    let now = Instant::now();
    let info = |time| CameraMessage::Information {
        time_boot_ms: time,
        flags: 64,
        device_id: 0,
        gimbal_id: 1,
    };
    apply(&mut state, 100, info(100), now);
    let initial = state.fc_reports.cameras[&(100, 0)].capability_revision;
    apply(&mut state, 100, info(100), now + Duration::from_secs(5));
    state.expire_camera_reports(now + Duration::from_secs(6));
    let expired = state.fc_reports.cameras[&(100, 0)];
    assert!(!expired.available);
    assert_eq!(expired.capability_revision, initial.wrapping_add(1));
    apply(&mut state, 100, info(101), now + Duration::from_secs(7));
    let restored = state.fc_reports.cameras[&(100, 0)];
    assert!(restored.available);
    assert_eq!(restored.capability_revision, initial.wrapping_add(2));
    assert_eq!(restored.epoch, expired.epoch);
}

#[test]
fn repeated_or_reordered_settings_never_refresh_observation_time() {
    let mut state = LinkState::default();
    let now = Instant::now();
    let setting = |time, value| CameraMessage::Settings {
        time_boot_ms: time,
        device_id: 0,
        travel_fraction: Some(value),
    };
    apply(&mut state, 100, setting(100, 0.25), now);
    apply(
        &mut state,
        100,
        setting(100, 0.75),
        now + Duration::from_secs(1),
    );
    apply(
        &mut state,
        100,
        setting(90, 0.9),
        now + Duration::from_secs(2),
    );
    let report = state.fc_reports.cameras[&(100, 0)];
    assert_eq!(report.travel_fraction, Some(0.25));
    assert_eq!(report.settings_received_at, Some(now));
}
