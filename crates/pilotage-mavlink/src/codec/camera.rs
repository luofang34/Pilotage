//! Standard camera discovery and settings decoding.

/// CAMERA_INFORMATION message identifier.
pub const CAMERA_INFORMATION_ID: u32 = 259;
/// CAMERA_SETTINGS message identifier.
pub const CAMERA_SETTINGS_ID: u32 = 260;
/// MAV_CMD_SET_CAMERA_ZOOM command identifier.
pub const MAV_CMD_SET_CAMERA_ZOOM: u16 = 531;

/// Camera reports preserve camera identity independently of vehicle estimates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraMessage {
    /// CAMERA_INFORMATION basic capability report.
    Information {
        /// Device boot clock in milliseconds.
        time_boot_ms: u32,
        /// CAMERA_CAP_FLAGS, not inferred from the camera model.
        flags: u32,
        /// Attached camera id, or zero for a MAVLink camera component.
        device_id: u8,
        /// Associated gimbal id; zero means unspecified.
        gimbal_id: u8,
    },
    /// CAMERA_SETTINGS lens travel; absent or NaN remains unknown.
    Settings {
        /// Device boot clock in milliseconds.
        time_boot_ms: u32,
        /// Attached camera id, or zero for a MAVLink camera component.
        device_id: u8,
        /// Lens travel fraction from wide to tele, when reported.
        travel_fraction: Option<f32>,
    },
}

pub(super) fn decode(id: u32, payload: &[u8]) -> Option<CameraMessage> {
    use super::decoding::{f32_at, u32_at};
    match id {
        CAMERA_INFORMATION_ID => Some(CameraMessage::Information {
            time_boot_ms: u32_at(payload, 0),
            flags: u32_at(payload, 20),
            gimbal_id: payload.get(235).copied().unwrap_or(0),
            device_id: payload.get(236).copied().unwrap_or(0),
        }),
        CAMERA_SETTINGS_ID => {
            // An absent extension does not establish a measured zero zoom.
            let travel_fraction = (payload.len() > 5)
                .then(|| f32_at(payload, 5) / 100.0)
                .filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
            Some(CameraMessage::Settings {
                time_boot_ms: u32_at(payload, 0),
                device_id: payload.get(13).copied().unwrap_or(0),
                travel_fraction,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
