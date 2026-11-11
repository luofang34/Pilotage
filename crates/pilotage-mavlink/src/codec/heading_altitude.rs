//! The heading and altitude setpoint, the airspeed command beside it, and
//! the one conversion of an intent altitude into the down coordinate of
//! the flight controller's local frame.
//!
//! A flight controller that holds heading, altitude, and airspeed with its
//! own sensors takes this setpoint with no position estimate. Each sender
//! of the setpoint uses these functions, so no sender has a conversion of
//! its own.

use pilotage_protocol::{AltitudeReference, HeadingAltitudeIntent};

use super::SET_POSITION_TARGET_ID;
use super::encoding::{encode_command_long, encode_frame_v2};

/// MAV_CMD_DO_CHANGE_SPEED.
pub const MAV_CMD_DO_CHANGE_SPEED: u16 = 178;

/// The `type_mask` of the setpoint: x and y (1, 2), the velocity (8, 16,
/// 32), the acceleration (64, 128, 256), and the yaw rate (2048) are
/// ignored. The down coordinate and the absolute yaw stay.
pub const HEADING_ALTITUDE_TYPE_MASK: u16 = 1 | 2 | 8 | 16 | 32 | 64 | 128 | 256 | 2048;

/// The speed type of [`MAV_CMD_DO_CHANGE_SPEED`] that names an airspeed.
const SPEED_TYPE_AIRSPEED: f32 = 0.0;
/// The throttle value of [`MAV_CMD_DO_CHANGE_SPEED`] that changes nothing.
const THROTTLE_NO_CHANGE: f32 = -1.0;

/// Why an intent altitude has no down coordinate in the local frame.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum LocalDownError {
    /// The altitude is above the ellipsoid and the height of the local
    /// origin above the ellipsoid has not been given.
    #[error(
        "the altitude {altitude_m} m is above the ellipsoid and the height of the local origin is not known"
    )]
    OriginHeightUnknown {
        /// The intent altitude, meters above the ellipsoid.
        altitude_m: f32,
    },
    /// The down coordinate is not a finite number.
    #[error("the altitude {altitude_m} m gives no finite down coordinate")]
    NotFinite {
        /// The intent altitude, meters.
        altitude_m: f32,
    },
}

/// The down coordinate, meters, of the altitude of `intent` in the local
/// frame of the flight controller.
///
/// The frame's origin is the flight controller's local origin and its
/// third axis points down. An altitude above the local origin is that
/// height with its sign changed. An altitude above the ellipsoid is first
/// made a height above the origin: `origin_ellipsoid_height_m` is the
/// height of the local origin above the same ellipsoid, as the host that
/// set the origin gives it. The flight controller's own origin report
/// cannot supply it: its altitude is above mean sea level.
///
/// # Errors
///
/// [`LocalDownError::OriginHeightUnknown`] for an altitude above the
/// ellipsoid when `origin_ellipsoid_height_m` is `None`, and
/// [`LocalDownError::NotFinite`] when the result is not a finite number.
pub fn local_down_m(
    intent: &HeadingAltitudeIntent,
    origin_ellipsoid_height_m: Option<f64>,
) -> Result<f32, LocalDownError> {
    let altitude_m = intent.altitude_m;
    let above_origin_m = match intent.reference {
        AltitudeReference::LocalOrigin => f64::from(altitude_m),
        AltitudeReference::Ellipsoid => {
            let origin_m = origin_ellipsoid_height_m
                .ok_or(LocalDownError::OriginHeightUnknown { altitude_m })?;
            f64::from(altitude_m) - origin_m
        }
    };
    let down_m = -above_origin_m as f32;
    if down_m.is_finite() {
        Ok(down_m)
    } else {
        Err(LocalDownError::NotFinite { altitude_m })
    }
}

/// Serializes a SET_POSITION_TARGET_LOCAL_NED setpoint that carries only
/// the down coordinate and the absolute yaw
/// ([`HEADING_ALTITUDE_TYPE_MASK`]). `yaw_rad` is the true heading.
///
/// Wire order as [`super::encode_velocity_setpoint`].
#[must_use]
pub fn encode_heading_altitude_setpoint(
    seq: u8,
    time_boot_ms: u32,
    down_m: f32,
    yaw_rad: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 65] {
    let mut payload = [0u8; 53];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    payload[12..16].copy_from_slice(&down_m.to_le_bytes());
    payload[40..44].copy_from_slice(&yaw_rad.to_le_bytes());
    payload[48..50].copy_from_slice(&HEADING_ALTITUDE_TYPE_MASK.to_le_bytes());
    payload[50] = target_system;
    payload[51] = target_component;
    payload[52] = 1; // MAV_FRAME_LOCAL_NED
    let mut frame = [0u8; 65];
    encode_frame_v2(seq, SET_POSITION_TARGET_ID, &payload, 143, &mut frame);
    frame
}

/// Serializes the COMMAND_LONG that sets the indicated airspeed the
/// flight controller holds, meters per second
/// ([`MAV_CMD_DO_CHANGE_SPEED`], speed type airspeed, throttle unchanged).
/// The airspeed stays in force until the next such command.
#[must_use]
pub fn encode_airspeed_command(
    seq: u8,
    airspeed_mps: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 45] {
    let params = [
        SPEED_TYPE_AIRSPEED,
        airspeed_mps,
        THROTTLE_NO_CHANGE,
        0.0,
        0.0,
        0.0,
        0.0,
    ];
    encode_command_long(
        seq,
        MAV_CMD_DO_CHANGE_SPEED,
        params,
        target_system,
        target_component,
    )
}

#[cfg(test)]
mod tests;
