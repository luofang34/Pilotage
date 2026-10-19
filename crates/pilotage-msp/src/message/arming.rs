//! Betaflight's arming-disable flags as typed reasons.

/// One reason Betaflight refuses to arm, by bit of `armingDisableFlags_e`
/// (`fc/runtime_config.h`). The CLI name of each reason is
/// [`ArmingBlocker::cli_name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArmingBlocker {
    /// No gyro was detected.
    NoGyro,
    /// Failsafe is active.
    Failsafe,
    /// The receiver signal is lost.
    RxLoss,
    /// The arm switch was on at boot or after a disarm; cycle it.
    NotDisarmed,
    /// The failsafe mode switch is on.
    BoxFailsafe,
    /// Runaway take-off prevention tripped.
    RunawayTakeoff,
    /// Crash detection tripped.
    CrashDetected,
    /// Throttle is above the arming limit.
    Throttle,
    /// The craft is tilted more than the small-angle limit.
    Angle,
    /// The boot grace time has not passed.
    BootGraceTime,
    /// A prearm switch is configured and off.
    NoPrearm,
    /// System load is too high.
    Load,
    /// Sensors are calibrating.
    Calibrating,
    /// The CLI is active.
    Cli,
    /// The OSD menu is active.
    CmsMenu,
    /// A BST device holds arming off.
    Bst,
    /// An MSP client holds arming off.
    Msp,
    /// The paralyze mode is active.
    Paralyze,
    /// GPS rescue needs a fix it does not have.
    Gps,
    /// The GPS rescue switch is on.
    RescueSwitch,
    /// DShot telemetry is required and missing.
    DshotTelemetry,
    /// A configuration change needs a reboot.
    RebootRequired,
    /// DShot bitbang is not ready.
    DshotBitbang,
    /// The accelerometer is not calibrated.
    AccCalibration,
    /// The motor protocol is not configured.
    MotorProtocol,
    /// The crash-flip switch is on.
    CrashFlip,
    /// The altitude hold switch is on.
    AltHoldSwitch,
    /// The position hold switch is on.
    PosHoldSwitch,
    /// The autopilot switch is on.
    AutopilotSwitch,
    /// The arm switch is on while another reason blocks arming.
    ArmSwitch,
}

impl ArmingBlocker {
    /// Every reason, in bit order.
    pub const ALL: [Self; 30] = [
        Self::NoGyro,
        Self::Failsafe,
        Self::RxLoss,
        Self::NotDisarmed,
        Self::BoxFailsafe,
        Self::RunawayTakeoff,
        Self::CrashDetected,
        Self::Throttle,
        Self::Angle,
        Self::BootGraceTime,
        Self::NoPrearm,
        Self::Load,
        Self::Calibrating,
        Self::Cli,
        Self::CmsMenu,
        Self::Bst,
        Self::Msp,
        Self::Paralyze,
        Self::Gps,
        Self::RescueSwitch,
        Self::DshotTelemetry,
        Self::RebootRequired,
        Self::DshotBitbang,
        Self::AccCalibration,
        Self::MotorProtocol,
        Self::CrashFlip,
        Self::AltHoldSwitch,
        Self::PosHoldSwitch,
        Self::AutopilotSwitch,
        Self::ArmSwitch,
    ];

    /// The bit this reason occupies in the flag word.
    #[must_use]
    pub fn bit(self) -> u32 {
        let index = Self::ALL
            .iter()
            .position(|&blocker| blocker == self)
            .unwrap_or_default();
        1 << index
    }

    /// The name Betaflight's CLI `status` prints for this reason.
    #[must_use]
    pub fn cli_name(self) -> &'static str {
        const NAMES: [&str; 30] = [
            "NOGYRO",
            "FAILSAFE",
            "RXLOSS",
            "NOT_DISARMED",
            "BOXFAILSAFE",
            "RUNAWAY",
            "CRASH",
            "THROTTLE",
            "ANGLE",
            "BOOTGRACE",
            "NOPREARM",
            "LOAD",
            "CALIB",
            "CLI",
            "CMS",
            "BST",
            "MSP",
            "PARALYZE",
            "GPS",
            "RESCUE_SW",
            "DSHOT_TELEM",
            "REBOOT_REQD",
            "DSHOT_BBANG",
            "NO_ACC_CAL",
            "MOTOR_PROTO",
            "FLIP_SWITCH",
            "ALT_HOLD_SW",
            "POS_HOLD_SW",
            "AUTOPILOT_SW",
            "ARM_SWITCH",
        ];
        Self::ALL
            .iter()
            .position(|&blocker| blocker == self)
            .and_then(|index| NAMES.get(index))
            .copied()
            .unwrap_or("UNKNOWN")
    }
}

/// The raw arming-disable flag word from `MSP_STATUS` / `MSP_STATUS_EX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArmingDisableFlags(pub u32);

impl ArmingDisableFlags {
    /// Whether no reason blocks arming.
    #[must_use]
    pub fn is_clear(self) -> bool {
        self.0 == 0
    }

    /// Whether `blocker` is set.
    #[must_use]
    pub fn contains(self, blocker: ArmingBlocker) -> bool {
        self.0 & blocker.bit() != 0
    }

    /// Every known reason that is set, in bit order.
    #[must_use]
    pub fn blockers(self) -> Vec<ArmingBlocker> {
        ArmingBlocker::ALL
            .into_iter()
            .filter(|&blocker| self.contains(blocker))
            .collect()
    }

    /// Set bits this crate has no name for (a newer firmware's reasons).
    #[must_use]
    pub fn unknown_bits(self) -> u32 {
        let known = ArmingBlocker::ALL
            .iter()
            .fold(0, |mask, blocker| mask | blocker.bit());
        self.0 & !known
    }
}
