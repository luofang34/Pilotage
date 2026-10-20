use super::{DjiHostError, parse_attachment};

#[test]
fn automatic_selection_conflicts_with_every_explicit_attachment() {
    for value in ["auto", "", "invalid"] {
        for explicit in ["12345", "", "invalid"] {
            assert!(matches!(
                parse_attachment(None, Some(explicit.into()), Some(value.into())),
                Err(DjiHostError::AttachmentConflict)
            ));
            assert!(matches!(
                parse_attachment(Some(explicit.into()), None, Some(value.into())),
                Err(DjiHostError::AttachmentConflict)
            ));
            assert!(matches!(
                parse_attachment(
                    Some(explicit.into()),
                    Some(explicit.into()),
                    Some(value.into())
                ),
                Err(DjiHostError::AttachmentConflict)
            ));
        }
    }
}

#[test]
fn automatic_selection_requires_the_exact_setting() {
    for value in ["", "AUTO", "auto ", "1", "true"] {
        assert!(matches!(
            parse_attachment(None, None, Some(value.into())),
            Err(DjiHostError::UsbSelection { .. })
        ));
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn automatic_selection_cannot_fall_back_to_serial_on_other_platforms() {
    assert!(matches!(
        parse_attachment(None, None, Some("auto".into())),
        Err(DjiHostError::UsbPlatform)
    ));
}

#[cfg(target_os = "macos")]
mod macos {
    use pilotage_adapter_dji::DjiAttachment;
    use pilotage_dji_usb::{UpperUsbCandidate, UsbError};

    use super::{DjiHostError, parse_attachment};

    #[test]
    fn automatic_selection_requires_exactly_one_candidate() {
        for ids in [vec![], vec![12, 34], vec![34, 12]] {
            let selection = parse_attachment(None, None, Some("auto".into())).expect("auto");
            let candidates = ids
                .iter()
                .map(|&registry_id| UpperUsbCandidate {
                    registry_id,
                    location_id: 0x110_0000,
                })
                .collect();
            let result = selection.resolve_with(|| Ok(candidates));
            match ids.as_slice() {
                [] => assert!(matches!(result, Err(DjiHostError::UsbNotFound))),
                _ => assert!(
                    matches!(result, Err(DjiHostError::UsbAmbiguous { registry_ids })
                    if registry_ids == ids)
                ),
            }
        }
        let selection = parse_attachment(None, None, Some("auto".into())).expect("auto");
        let attachment = selection
            .resolve_with(|| {
                Ok(vec![UpperUsbCandidate {
                    registry_id: 34,
                    location_id: 0x110_0000,
                }])
            })
            .expect("unique candidate");
        assert!(matches!(
            attachment,
            DjiAttachment::MacUsb { registry_id: 34 }
        ));
    }

    #[test]
    fn incomplete_discovery_is_an_error_with_the_native_source() {
        let selection = parse_attachment(None, None, Some("auto".into())).expect("auto");
        let error = selection
            .resolve_with(|| {
                Err(UsbError::Native {
                    stage: 21,
                    status: 5,
                })
            })
            .expect_err("discovery error");
        assert!(matches!(
            error,
            DjiHostError::UsbDiscovery(UsbError::Native {
                stage: 21,
                status: 5
            })
        ));
        assert!(
            std::error::Error::source(&error)
                .and_then(|source| source.downcast_ref::<UsbError>())
                .is_some()
        );
    }

    #[test]
    fn an_explicit_identity_does_not_enumerate_or_select_another_device() {
        let selection = parse_attachment(None, Some("12345".into()), None).expect("identity");
        let attachment = selection
            .resolve_with(|| Err(UsbError::InvalidIdentity))
            .expect("explicit selector skips discovery");
        assert!(matches!(
            attachment,
            DjiAttachment::MacUsb { registry_id: 12345 }
        ));
    }
}
