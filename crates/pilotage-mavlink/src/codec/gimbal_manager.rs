//! Manager ownership from the standard GIMBAL_MANAGER_STATUS message.

/// GIMBAL_MANAGER_STATUS message identifier.
pub const GIMBAL_MANAGER_STATUS_ID: u32 = 281;

/// Reported manager ownership, independent of a locally queued claim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GimbalManagerStatus {
    /// Manager boot clock, in milliseconds.
    pub time_boot_ms: u32,
    /// Actual manager flags currently applied.
    pub flags: u32,
    /// Managed device component id or non-MAVLink device id.
    pub device_id: u8,
    /// Current primary holder's system id; zero means none.
    pub primary_system: u8,
    /// Current primary holder's component id; zero means none.
    pub primary_component: u8,
}

pub(super) fn decode(payload: &[u8]) -> GimbalManagerStatus {
    GimbalManagerStatus {
        time_boot_ms: super::decoding::u32_at(payload, 0),
        flags: super::decoding::u32_at(payload, 4),
        device_id: payload.get(8).copied().unwrap_or(0),
        primary_system: payload.get(9).copied().unwrap_or(0),
        primary_component: payload.get(10).copied().unwrap_or(0),
    }
}
