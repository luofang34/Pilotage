use std::ffi::OsString;
use std::path::Path;

use pilotage_adapter_dji::{DjiAttachment, DjiControlMode};

use super::{
    AttachmentSelection, CAPTURE, DjiHostError, SERIAL, config_from_values, parse_attachment,
    parse_decoder,
};

#[test]
fn the_attachment_and_capture_are_explicit() {
    assert!(matches!(
        config_from_values(None, None, None, None, None, None, None),
        Err(DjiHostError::MissingAttachment)
    ));
    assert!(matches!(
        config_from_values(
            Some(device_path().into()),
            None,
            None,
            None,
            None,
            None,
            None
        ),
        Err(DjiHostError::Missing { variable: CAPTURE })
    ));
    assert!(matches!(
        parse_attachment(Some(OsString::new()), None, None),
        Err(DjiHostError::Missing { variable: SERIAL })
    ));
}

#[test]
fn transport_settings_conflict_even_when_one_is_empty_or_invalid() {
    for serial in [device_path(), "", "/tmp/unrelated"] {
        for registry_id in ["12345", "", "invalid"] {
            assert!(matches!(
                parse_attachment(Some(serial.into()), Some(registry_id.into()), None),
                Err(DjiHostError::AttachmentConflict)
            ));
        }
    }
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
            parse_attachment(Some(serial.into()), None, None),
            Err(DjiHostError::SerialPath { .. })
        ));
    }
    for capture in ["capture.jsonl", "/dev/cu.usbmodemC5", "/"] {
        assert!(matches!(
            config_from_values(
                Some(device_path().into()),
                None,
                None,
                Some(capture.into()),
                None,
                None,
                None
            ),
            Err(DjiHostError::CapturePath { .. })
        ));
    }
}

#[test]
fn serial_configuration_reaches_the_adapter_without_opening_the_device() {
    let config = config_from_values(
        Some(device_path().into()),
        None,
        None,
        Some("/tmp/capture.jsonl".into()),
        None,
        None,
        None,
    )
    .expect("explicit configuration");
    assert!(
        matches!(config.attachment, AttachmentSelection::Explicit(DjiAttachment::Serial { path })
        if path == Path::new(device_path()))
    );
    assert_eq!(config.capture_path, Path::new("/tmp/capture.jsonl"));
    assert_eq!(config.ffmpeg, Path::new("ffmpeg"));
}

#[cfg(target_os = "macos")]
#[test]
fn usb_configuration_preserves_the_exact_current_registry_identity() {
    for identity in ["4294971952", "0x100001230"] {
        let config = config_from_values(
            None,
            Some(identity.into()),
            None,
            Some("/tmp/capture.jsonl".into()),
            None,
            Some("/opt/homebrew/bin/ffmpeg".into()),
            None,
        )
        .expect("explicit USB configuration");
        assert!(matches!(
            config.attachment,
            AttachmentSelection::Explicit(DjiAttachment::MacUsb {
                registry_id: 0x1_0000_1230
            })
        ));
        assert_eq!(config.ffmpeg, Path::new("/opt/homebrew/bin/ffmpeg"));
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn usb_configuration_on_another_platform_has_no_serial_fallback() {
    assert!(matches!(
        parse_attachment(None, Some("12345".into()), None),
        Err(DjiHostError::UsbPlatform)
    ));
}

#[test]
fn invalid_registry_identity_cannot_select_another_device() {
    for value in [
        "", "0", "0x0", "-1", "+1", "1.0", "1 ", " 1", "0x", "0X12", "0xGG",
    ] {
        assert!(matches!(
            parse_attachment(None, Some(value.into()), None),
            Err(DjiHostError::RegistryId { .. })
        ));
    }
    use std::os::unix::ffi::OsStringExt;
    assert!(matches!(
        parse_attachment(None, Some(OsString::from_vec(vec![0xff])), None),
        Err(DjiHostError::RegistryId { .. })
    ));
    assert!(
        matches!(parse_attachment(None, Some("18446744073709551616".into()), None),
        Err(DjiHostError::RegistryIdRange { source, .. })
            if source.kind() == &std::num::IntErrorKind::PosOverflow)
    );
}

#[test]
fn decoder_configuration_is_explicit_when_present() {
    for path in ["", ".", "..", "/", "./ffmpeg", "tools/ffmpeg", "ffmpeg/"] {
        assert!(matches!(
            parse_decoder(Some(path.into())),
            Err(DjiHostError::DecoderPath { .. })
        ));
    }
    for path in ["ffmpeg", "ffmpeg-custom", "/opt/homebrew/bin/ffmpeg"] {
        assert_eq!(
            parse_decoder(Some(path.into())).expect("decoder"),
            Path::new(path)
        );
    }
}

#[test]
fn one_way_control_requires_an_explicit_mode() {
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
            None,
            None,
            Some("/tmp/capture.jsonl".into()),
            value.map(Into::into),
            None,
            None,
        )
        .expect("valid explicit configuration");
        assert_eq!(config.control_mode, expected);
    }
}

#[test]
fn experimental_flight_requires_an_exact_opt_in() {
    let configured = |value| {
        config_from_values(
            Some(device_path().into()),
            None,
            None,
            Some("/tmp/capture.jsonl".into()),
            None,
            None,
            value,
        )
    };
    assert!(
        !configured(None)
            .expect("default disabled")
            .experimental_flight
    );
    assert!(
        configured(Some("1".into()))
            .expect("explicit opt-in")
            .experimental_flight
    );
    for value in ["", "0", "true", "yes", "1 ", " 1"] {
        assert!(matches!(
            configured(Some(value.into())),
            Err(DjiHostError::ExperimentalFlight { .. })
        ));
    }
}

#[test]
fn an_unknown_mode_cannot_enable_control() {
    for value in ["", "bench", "true", "observed ", "BENCH-INITIAL-HORIZONTAL"] {
        assert!(matches!(
            config_from_values(
                Some(device_path().into()),
                None,
                None,
                Some("/tmp/capture.jsonl".into()),
                Some(value.into()),
                None,
                None
            ),
            Err(DjiHostError::ControlMode { .. })
        ));
    }
}

#[cfg(test)]
fn device_path() -> &'static str {
    if cfg!(target_os = "macos") {
        "/dev/cu.usbmodemC5"
    } else {
        "/dev/ttyACM0"
    }
}

#[test]
fn virtual_stick_requires_its_own_exact_opt_in() {
    assert!(!super::parse_virtual_stick(None).unwrap());
    assert!(super::parse_virtual_stick(Some("1".into())).unwrap());
    for value in ["", "0", "true", "1 "] {
        assert!(matches!(
            super::parse_virtual_stick(Some(value.into())),
            Err(DjiHostError::VirtualStick { .. })
        ));
    }
}
