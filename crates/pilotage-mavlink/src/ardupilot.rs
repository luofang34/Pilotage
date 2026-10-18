//! ArduPilot vocabulary carried inside standard MAVLink fields: the
//! ArduCopter flight-mode numbers of HEARTBEAT `custom_mode`.

/// An ArduCopter flight mode, decoded from HEARTBEAT `custom_mode`
/// (ArduPilot `ArduCopter/mode.h`, `Mode::Number`). A number this
/// vocabulary does not name is kept as [`ArduCopterMode::Other`] rather
/// than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArduCopterMode {
    /// Manual angle, manual throttle.
    Stabilize,
    /// Manual body rate, manual throttle.
    Acro,
    /// Manual angle, automatic altitude.
    AltHold,
    /// Mission waypoints.
    Auto,
    /// GCS-commanded position, velocity, or takeoff.
    Guided,
    /// Pilot-commanded position hold.
    Loiter,
    /// Return to launch.
    Rtl,
    /// Orbit a point.
    Circle,
    /// Land in place.
    Land,
    /// Semi-autonomous drift.
    Drift,
    /// Earth-frame rate, manual throttle.
    Sport,
    /// Automatic roll flip.
    Flip,
    /// Gain tuning.
    AutoTune,
    /// Position hold with manual override.
    PosHold,
    /// Full brake.
    Brake,
    /// Throw to launch.
    Throw,
    /// ADS-B avoidance.
    AvoidAdsb,
    /// Guided without GPS (attitude and altitude only).
    GuidedNoGps,
    /// Return along the flown path.
    SmartRtl,
    /// Optical-flow position hold.
    FlowHold,
    /// Follow another vehicle.
    Follow,
    /// Zigzag between two points.
    ZigZag,
    /// System identification.
    SystemId,
    /// Autonomous autorotation.
    Autorotate,
    /// Mission landing sequence.
    AutoRtl,
    /// Flip over after a crash.
    Turtle,
    /// A mode number this vocabulary does not name.
    Other(u32),
}

impl ArduCopterMode {
    /// Decodes HEARTBEAT `custom_mode`.
    #[must_use]
    pub fn from_custom_mode(custom_mode: u32) -> Self {
        match custom_mode {
            0 => Self::Stabilize,
            1 => Self::Acro,
            2 => Self::AltHold,
            3 => Self::Auto,
            4 => Self::Guided,
            5 => Self::Loiter,
            6 => Self::Rtl,
            7 => Self::Circle,
            9 => Self::Land,
            11 => Self::Drift,
            13 => Self::Sport,
            14 => Self::Flip,
            15 => Self::AutoTune,
            16 => Self::PosHold,
            17 => Self::Brake,
            18 => Self::Throw,
            19 => Self::AvoidAdsb,
            20 => Self::GuidedNoGps,
            21 => Self::SmartRtl,
            22 => Self::FlowHold,
            23 => Self::Follow,
            24 => Self::ZigZag,
            25 => Self::SystemId,
            26 => Self::Autorotate,
            27 => Self::AutoRtl,
            28 => Self::Turtle,
            other => Self::Other(other),
        }
    }

    /// The `custom_mode` number, as MAV_CMD_DO_SET_MODE param 2 takes it.
    #[must_use]
    pub fn custom_mode(self) -> u32 {
        match self {
            Self::Stabilize => 0,
            Self::Acro => 1,
            Self::AltHold => 2,
            Self::Auto => 3,
            Self::Guided => 4,
            Self::Loiter => 5,
            Self::Rtl => 6,
            Self::Circle => 7,
            Self::Land => 9,
            Self::Drift => 11,
            Self::Sport => 13,
            Self::Flip => 14,
            Self::AutoTune => 15,
            Self::PosHold => 16,
            Self::Brake => 17,
            Self::Throw => 18,
            Self::AvoidAdsb => 19,
            Self::GuidedNoGps => 20,
            Self::SmartRtl => 21,
            Self::FlowHold => 22,
            Self::Follow => 23,
            Self::ZigZag => 24,
            Self::SystemId => 25,
            Self::Autorotate => 26,
            Self::AutoRtl => 27,
            Self::Turtle => 28,
            Self::Other(number) => number,
        }
    }
}

#[cfg(test)]
mod tests;
