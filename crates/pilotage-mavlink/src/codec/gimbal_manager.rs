//! Standard gimbal-manager discovery and ownership reports.

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

/// GIMBAL_MANAGER_INFORMATION message identifier.
pub const GIMBAL_MANAGER_INFORMATION_ID: u32 = 280;

/// Operations and limits declared by one manager for one device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GimbalManagerInformation {
    /// Manager boot clock, in milliseconds.
    pub time_boot_ms: u32,
    /// GIMBAL_MANAGER_CAP_FLAGS; device flags cannot substitute for these.
    pub cap_flags: u32,
    /// Minimum roll, radians; NaN when unknown.
    pub roll_min: f32,
    /// Maximum roll, radians; NaN when unknown.
    pub roll_max: f32,
    /// Minimum pitch, radians; NaN when unknown.
    pub pitch_min: f32,
    /// Maximum pitch, radians; NaN when unknown.
    pub pitch_max: f32,
    /// Minimum yaw, radians; NaN when unknown.
    pub yaw_min: f32,
    /// Maximum yaw, radians; NaN when unknown.
    pub yaw_max: f32,
    /// Managed component ID, or attached non-MAVLink device ID 1..6.
    pub device_id: u8,
}

impl GimbalManagerInformation {
    pub(crate) fn same_contract(self, other: Self) -> bool {
        self.cap_flags == other.cap_flags
            && self.device_id == other.device_id
            && [
                self.roll_min,
                self.roll_max,
                self.pitch_min,
                self.pitch_max,
                self.yaw_min,
                self.yaw_max,
            ]
            .map(f32::to_bits)
                == [
                    other.roll_min,
                    other.roll_max,
                    other.pitch_min,
                    other.pitch_max,
                    other.yaw_min,
                    other.yaw_max,
                ]
                .map(f32::to_bits)
    }
}

pub(super) fn decode_information(payload: &[u8]) -> GimbalManagerInformation {
    use super::decoding::{f32_at, u32_at};
    GimbalManagerInformation {
        time_boot_ms: u32_at(payload, 0),
        cap_flags: u32_at(payload, 4),
        roll_min: f32_at(payload, 8),
        roll_max: f32_at(payload, 12),
        pitch_min: f32_at(payload, 16),
        pitch_max: f32_at(payload, 20),
        yaw_min: f32_at(payload, 24),
        yaw_max: f32_at(payload, 28),
        device_id: payload.get(32).copied().unwrap_or(0),
    }
}
