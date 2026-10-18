//! How to read a gimbal device's reported orientation, independent of
//! the flight-controller family (`pilotage.v1.GimbalAttitude` fields
//! `yaw_frame` and the lock fields).
//!
//! An adapter translates its device's own mode bits into these types, so
//! no client decodes them.

use crate::wire;

/// The frame that a gimbal yaw angle is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GimbalYawFrame {
    /// Yaw is relative to the vehicle's forward axis.
    Vehicle,
    /// Yaw is relative to true north.
    Earth,
}

impl GimbalYawFrame {
    /// The wire enum for this frame.
    #[must_use]
    pub const fn to_wire(self) -> wire::GimbalYawFrame {
        match self {
            Self::Vehicle => wire::GimbalYawFrame::Vehicle,
            Self::Earth => wire::GimbalYawFrame::Earth,
        }
    }

    /// The frame a raw wire value carries. `None` for
    /// `GIMBAL_YAW_FRAME_UNSPECIFIED` and for a value this build does not
    /// know: the yaw then has no frame a client may show it against.
    #[must_use]
    pub fn from_wire(raw: i32) -> Option<Self> {
        match wire::GimbalYawFrame::try_from(raw).ok()? {
            wire::GimbalYawFrame::Unspecified => None,
            wire::GimbalYawFrame::Vehicle => Some(Self::Vehicle),
            wire::GimbalYawFrame::Earth => Some(Self::Earth),
        }
    }
}

/// Which axes the device holds fixed relative to the Earth. An axis that
/// is not locked follows the vehicle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GimbalAxisLocks {
    /// The device holds roll relative to the horizon.
    pub roll: bool,
    /// The device holds pitch relative to the horizon.
    pub pitch: bool,
    /// The device holds yaw relative to north.
    pub yaw: bool,
}

/// How to read one gimbal orientation report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GimbalReference {
    /// The frame that the reported yaw is relative to.
    pub yaw_frame: GimbalYawFrame,
    /// The axes the device holds fixed relative to the Earth.
    pub locks: GimbalAxisLocks,
}

#[cfg(test)]
mod tests;
