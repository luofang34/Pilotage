//! Translation of MAVLink `GIMBAL_DEVICE_FLAGS` into the protocol's typed
//! gimbal reference, so the bits never cross the adapter boundary
//! (ADR-0008).

use pilotage_adapter_api::{GimbalAxisLocks, GimbalReference, GimbalYawFrame};

const ROLL_LOCK: u16 = 4;
const PITCH_LOCK: u16 = 8;
const YAW_LOCK: u16 = 16;
const YAW_IN_VEHICLE_FRAME: u16 = 32;
const YAW_IN_EARTH_FRAME: u16 = 64;

/// How to read a `GIMBAL_DEVICE_ATTITUDE_STATUS` orientation with these
/// device flags.
///
/// An explicit yaw-frame bit wins. A device that sets neither frame bit
/// is an older Gimbal Protocol v2 device: its yaw is earth-relative when
/// it locks yaw and vehicle-relative when it does not, as the MAVLink
/// definition of the message states.
#[must_use]
pub const fn gimbal_reference(flags: u16) -> GimbalReference {
    let yaw_lock = flags & YAW_LOCK != 0;
    let yaw_frame = if flags & YAW_IN_EARTH_FRAME != 0 {
        GimbalYawFrame::Earth
    } else if flags & YAW_IN_VEHICLE_FRAME != 0 {
        GimbalYawFrame::Vehicle
    } else if yaw_lock {
        GimbalYawFrame::Earth
    } else {
        GimbalYawFrame::Vehicle
    };
    GimbalReference {
        yaw_frame,
        locks: GimbalAxisLocks {
            roll: flags & ROLL_LOCK != 0,
            pitch: flags & PITCH_LOCK != 0,
            yaw: yaw_lock,
        },
    }
}

#[cfg(test)]
mod tests;
