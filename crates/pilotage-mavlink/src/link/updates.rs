//! Per-group cache entries the link task writes and adapters sample:
//! each carries its receive stamp so staleness propagates to consumers
//! (ADR-0018: loss of data marks groups stale rather than freezing
//! them).

use std::time::Instant;

use pilotage_adapter_api::MeasurementStamp;

/// One attitude update with its receive stamp.
#[derive(Debug, Clone, Copy)]
pub struct AttitudeUpdate {
    /// Quaternion (w, x, y, z), body FRD → world NED.
    pub quat_wxyz: [f32; 4],
    /// Body rates (p, q, r) rad/s.
    pub rates_rps: [f32; 3],
    /// Milliseconds since FC boot.
    pub time_boot_ms: u32,
    /// Identity and acquisition stamp for this group update.
    pub stamp: MeasurementStamp,
    /// Authorization bits retained for this numeric acquisition.
    pub valid_flags: u32,
    /// Canonical quality retained for this numeric acquisition.
    pub quality: u32,
    /// When this update was received.
    pub received_at: Instant,
}

/// One kinematics update with its receive stamp.
#[derive(Debug, Clone, Copy)]
pub struct KinematicsUpdate {
    /// Position NED, meters.
    pub pos_ned_m: [f32; 3],
    /// Velocity NED, m/s.
    pub vel_ned_mps: [f32; 3],
    /// Milliseconds since FC boot.
    pub time_boot_ms: u32,
    /// Identity and acquisition stamp for this group update.
    pub stamp: MeasurementStamp,
    /// Authorization bits retained for this numeric acquisition.
    pub valid_flags: u32,
    /// Canonical quality retained for this numeric acquisition.
    pub quality: u32,
    /// When this update was received.
    pub received_at: Instant,
}

/// Latest gimbal-device orientation (Gimbal Protocol v2 attitude
/// status), with its receive stamp for staleness handling.
#[derive(Debug, Clone, Copy)]
pub struct GimbalDeviceAttitude {
    /// Orientation quaternion (w, x, y, z); vehicle-frame yaw unless
    /// the device flags say YAW_LOCK.
    pub quat_wxyz: [f32; 4],
    /// Angular velocity (x, y, z) rad/s; NaN when the device does not
    /// report rates.
    pub rates_rps: [f32; 3],
    /// Milliseconds since gimbal/FC boot.
    pub time_boot_ms: u32,
    /// GIMBAL_DEVICE_FLAGS in effect.
    pub flags: u16,
    /// Non-zero reports a device failure condition.
    pub failure_flags: u32,
    /// When this report was received.
    pub received_at: Instant,
}

/// One command acknowledgement with its receive stamp.
#[derive(Debug, Clone, Copy)]
pub struct CommandAckReport {
    /// The acknowledged MAV_CMD id.
    pub command: u16,
    /// MAV_RESULT (0 = accepted).
    pub result: u8,
    /// When the acknowledgement was received.
    pub received_at: Instant,
}
