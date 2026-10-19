use super::{CAPTURE, DjiHostError, SERIAL, config_from_values};

#[test]
fn the_device_and_capture_are_explicit() {
    assert!(matches!(
        config_from_values(None, None),
        Err(DjiHostError::Missing { variable: SERIAL })
    ));
    assert!(matches!(
        config_from_values(Some("/dev/cu.usbmodemC5".into()), None),
        Err(DjiHostError::Missing { variable: CAPTURE })
    ));
}

#[test]
fn another_device_or_a_relative_path_is_refused() {
    for serial in [
        "/dev/cu.debug-console",
        "/dev/cu.Bluetooth-Incoming-Port",
        "cu.usbmodemC5",
        "/tmp/cu.usbmodemC5",
        "/dev/cu.usbmodem",
    ] {
        assert!(matches!(
            config_from_values(Some(serial.into()), Some("/tmp/capture.jsonl".into())),
            Err(DjiHostError::SerialPath { .. })
        ));
    }
    for capture in ["capture.jsonl", "/dev/cu.usbmodemC5", "/"] {
        assert!(matches!(
            config_from_values(Some("/dev/cu.usbmodemC5".into()), Some(capture.into())),
            Err(DjiHostError::CapturePath { .. })
        ));
    }
}

#[test]
fn valid_paths_reach_the_adapter_without_opening_the_device() {
    let config = config_from_values(
        Some("/dev/cu.usbmodemC5".into()),
        Some("/tmp/capture.jsonl".into()),
    );
    assert!(config.is_ok());
    if let Ok(config) = config {
        assert_eq!(
            config.serial_path,
            std::path::Path::new("/dev/cu.usbmodemC5")
        );
        assert_eq!(
            config.capture_path,
            std::path::Path::new("/tmp/capture.jsonl")
        );
    }
}
