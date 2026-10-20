use super::*;

#[test]
fn discovery_reads_camera_identity_and_capability_flags() {
    let mut payload = [0_u8; 237];
    payload[..4].copy_from_slice(&1234_u32.to_le_bytes());
    payload[20..24].copy_from_slice(&64_u32.to_le_bytes());
    payload[235] = 2;
    payload[236] = 3;
    assert_eq!(
        decode(CAMERA_INFORMATION_ID, &payload),
        Some(CameraMessage::Information {
            time_boot_ms: 1234,
            flags: 64,
            device_id: 3,
            gimbal_id: 2,
        })
    );
}

#[test]
fn camera_settings_preserve_unknown_and_percent_units() {
    let mut payload = [0_u8; 14];
    payload[5..9].copy_from_slice(&25.0_f32.to_le_bytes());
    payload[13] = 2;
    assert_eq!(
        decode(CAMERA_SETTINGS_ID, &payload),
        Some(CameraMessage::Settings {
            time_boot_ms: 0,
            device_id: 2,
            travel_fraction: Some(0.25),
        })
    );
    payload[5..9].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(matches!(
        decode(CAMERA_SETTINGS_ID, &payload),
        Some(CameraMessage::Settings {
            travel_fraction: None,
            ..
        })
    ));
    assert!(matches!(
        decode(CAMERA_SETTINGS_ID, &payload[..5]),
        Some(CameraMessage::Settings {
            travel_fraction: None,
            ..
        })
    ));
}
