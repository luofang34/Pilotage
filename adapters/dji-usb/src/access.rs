/// Native connection access state; this does not assert aircraft or control readiness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbAccessStatus {
    /// No eligible upper-port connection is present.
    Waiting,
    /// macOS requires authorization or unlock before access can proceed.
    AuthorizationRequired,
    /// More than one eligible physical connection requires selection.
    Ambiguous,
    /// A bounded handshake or its data-device enumeration is in progress.
    Handshake,
    /// A host data-device profile is present; opening must still validate it.
    DataInterfacePresent,
    /// This connection's attempt ended; a deliberate recovery or replug is needed.
    ReconnectRequired,
    /// Access ownership or cleanup cannot be established safely.
    Blocked,
}

impl UsbAccessStatus {
    #[cfg(target_os = "macos")]
    pub(crate) fn from_native(value: u8) -> Self {
        match value {
            0 => Self::Waiting,
            1 => Self::AuthorizationRequired,
            2 => Self::Ambiguous,
            3 => Self::Handshake,
            4 => Self::DataInterfacePresent,
            5 => Self::ReconnectRequired,
            _ => Self::Blocked,
        }
    }
}
