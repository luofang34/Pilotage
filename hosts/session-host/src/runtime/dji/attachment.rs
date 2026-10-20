use std::ffi::OsString;
use std::path::Path;

use pilotage_adapter_dji::DjiAttachment;

use super::{DjiHostError, SERIAL, required_path};

pub(super) enum AttachmentSelection {
    Explicit(DjiAttachment),
    #[cfg(target_os = "macos")]
    AutoUsb,
}

impl AttachmentSelection {
    pub(super) fn resolve_blocking(self) -> Result<DjiAttachment, DjiHostError> {
        #[cfg(target_os = "macos")]
        {
            self.resolve_with(pilotage_dji_usb::discover_upper_blocking)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let Self::Explicit(attachment) = self;
            Ok(attachment)
        }
    }

    #[cfg(target_os = "macos")]
    fn resolve_with(
        self,
        discover: impl FnOnce() -> Result<
            Vec<pilotage_dji_usb::UpperUsbCandidate>,
            pilotage_dji_usb::UsbError,
        >,
    ) -> Result<DjiAttachment, DjiHostError> {
        match self {
            Self::Explicit(attachment) => Ok(attachment),
            Self::AutoUsb => {
                let candidates = discover().map_err(DjiHostError::UsbDiscovery)?;
                match candidates.as_slice() {
                    [candidate] => Ok(DjiAttachment::MacUsb {
                        registry_id: candidate.registry_id,
                    }),
                    [] => Err(DjiHostError::UsbNotFound),
                    _ => Err(DjiHostError::UsbAmbiguous {
                        registry_ids: candidates.iter().map(|entry| entry.registry_id).collect(),
                    }),
                }
            }
        }
    }
}

pub(super) fn parse_attachment(
    serial: Option<OsString>,
    usb_registry_id: Option<OsString>,
    usb: Option<OsString>,
) -> Result<AttachmentSelection, DjiHostError> {
    match (serial, usb_registry_id, usb) {
        (None, None, None) => Err(DjiHostError::MissingAttachment),
        (Some(serial), None, None) => {
            let path = required_path(Some(serial), SERIAL)?;
            let prefix = if cfg!(target_os = "macos") {
                "cu.usbmodem"
            } else {
                "ttyACM"
            };
            if path.parent() != Some(Path::new("/dev"))
                || !path.file_name().is_some_and(|name| {
                    name.to_str().is_some_and(|name| {
                        name.strip_prefix(prefix)
                            .is_some_and(|suffix| !suffix.is_empty())
                    })
                })
            {
                return Err(DjiHostError::SerialPath { path });
            }
            Ok(AttachmentSelection::Explicit(DjiAttachment::Serial {
                path,
            }))
        }
        (None, Some(value), None) => parse_usb_attachment(value).map(AttachmentSelection::Explicit),
        (None, None, Some(value)) => parse_auto(value),
        _ => Err(DjiHostError::AttachmentConflict),
    }
}

fn parse_auto(value: OsString) -> Result<AttachmentSelection, DjiHostError> {
    if value != "auto" {
        return Err(DjiHostError::UsbSelection { value });
    }
    #[cfg(target_os = "macos")]
    {
        Ok(AttachmentSelection::AutoUsb)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(DjiHostError::UsbPlatform)
    }
}

fn parse_usb_attachment(value: OsString) -> Result<DjiAttachment, DjiHostError> {
    let Some(text) = value.to_str() else {
        return Err(DjiHostError::RegistryId { value });
    };
    let (digits, radix) = text.strip_prefix("0x").map_or((text, 10), |hex| (hex, 16));
    let valid_digits = digits.bytes().all(|byte| {
        if radix == 16 {
            byte.is_ascii_hexdigit()
        } else {
            byte.is_ascii_digit()
        }
    });
    if digits.is_empty() || !valid_digits {
        return Err(DjiHostError::RegistryId { value });
    }
    let registry_id =
        u64::from_str_radix(digits, radix).map_err(|source| DjiHostError::RegistryIdRange {
            value: value.clone(),
            source,
        })?;
    if registry_id == 0 {
        return Err(DjiHostError::RegistryId { value });
    }
    #[cfg(target_os = "macos")]
    {
        Ok(DjiAttachment::MacUsb { registry_id })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _registry_id = registry_id;
        Err(DjiHostError::UsbPlatform)
    }
}

#[cfg(test)]
mod tests;
