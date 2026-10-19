use super::{CAPTURE, DjiHostError, SERIAL, config_from_values};

#[test]
fn the_device_and_capture_are_explicit() {
    assert!(matches!(
        config_from_values(None, None, None),
        Err(DjiHostError::Missing { variable: SERIAL })
    ));
    assert!(matches!(
        config_from_values(Some(device_path().into()), None, None),
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
            config_from_values(Some(serial.into()), Some("/tmp/capture.jsonl".into()), None),
            Err(DjiHostError::SerialPath { .. })
        ));
    }
    for capture in ["capture.jsonl", "/dev/cu.usbmodemC5", "/"] {
        assert!(matches!(
            config_from_values(Some(device_path().into()), Some(capture.into()), None),
            Err(DjiHostError::CapturePath { .. })
        ));
    }
}

#[test]
fn valid_paths_reach_the_adapter_without_opening_the_device() {
    let config = config_from_values(
        Some(device_path().into()),
        Some("/tmp/capture.jsonl".into()),
        None,
    );
    assert!(config.is_ok());
    if let Ok(config) = config {
        assert_eq!(config.serial_path, std::path::Path::new(device_path()));
        assert_eq!(
            config.capture_path,
            std::path::Path::new("/tmp/capture.jsonl")
        );
    }
}

#[test]
fn one_way_control_requires_an_explicit_mode() {
    use pilotage_adapter_dji::DjiControlMode;

    for (value, expected) in [
        (None, DjiControlMode::Observed),
        (Some("observed"), DjiControlMode::Observed),
        (
            Some("bench-initial-horizontal"),
            DjiControlMode::BenchInitialHorizontal,
        ),
    ] {
        let config = config_from_values(
            Some(device_path().into()),
            Some("/tmp/capture.jsonl".into()),
            value.map(Into::into),
        )
        .expect("valid explicit configuration");
        assert_eq!(config.control_mode, expected);
    }
}

#[test]
fn an_unknown_mode_cannot_enable_control() {
    for value in ["", "bench", "true", "observed ", "BENCH-INITIAL-HORIZONTAL"] {
        assert!(matches!(
            config_from_values(
                Some(device_path().into()),
                Some("/tmp/capture.jsonl".into()),
                Some(value.into()),
            ),
            Err(DjiHostError::ControlMode { .. })
        ));
    }
}

fn device_path() -> &'static str {
    if cfg!(target_os = "macos") {
        "/dev/cu.usbmodemC5"
    } else {
        "/dev/ttyACM0"
    }
}
