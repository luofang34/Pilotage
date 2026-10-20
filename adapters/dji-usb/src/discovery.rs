use crate::UsbError;

/// A device whose registry properties match the upper C5 USB profile.
///
/// This is a read-only snapshot. The device can disconnect or change before
/// it opens. Opening the selected identity checks the full USB descriptors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpperUsbCandidate {
    /// The current attachment's I/O Registry identity.
    pub registry_id: u64,
    /// The physical USB location reported by I/O Registry.
    pub location_id: u32,
}

/// Read all upper C5 candidates from I/O Registry without opening a USB device.
///
/// This does not change USB roles or configurations. An empty list means that
/// no matching host profile is present. Multiple entries require an explicit
/// selection. Neither result starts a bootstrap operation.
pub fn discover_upper_blocking() -> Result<Vec<UpperUsbCandidate>, UsbError> {
    #[cfg(target_os = "macos")]
    {
        crate::ffi::discover()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(UsbError::UnsupportedPlatform)
    }
}
