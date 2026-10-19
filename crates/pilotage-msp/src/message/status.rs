//! `MSP_STATUS` and `MSP_STATUS_EX`.

use super::cursor::Cursor;
use super::{ArmingDisableFlags, ModeFlags, PayloadError};

/// The flight-controller status both status commands report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// PID loop period, microseconds.
    pub cycle_time_us: u16,
    /// Sensor presence bits: acc 0, baro 1, mag 2, GPS 3, rangefinder 4,
    /// gyro 5.
    pub sensors: u16,
    /// Active modes, one bit per box in `MSP_BOXIDS` order.
    pub modes: ModeFlags,
    /// The selected PID profile.
    pub pid_profile: u8,
    /// Average system load, percent times ten.
    pub system_load: u16,
    /// Why arming is refused right now.
    pub arming_disable: ArmingDisableFlags,
    /// Whether a configuration change waits for a reboot.
    pub reboot_required: bool,
}

/// Sensor presence bit for the GNSS receiver.
const SENSOR_GPS: u16 = 1 << 3;

impl Status {
    /// Whether the flight controller reports a working GNSS receiver.
    #[must_use]
    pub fn has_gps(&self) -> bool {
        self.sensors & SENSOR_GPS != 0
    }

    /// Decodes an `MSP_STATUS` or `MSP_STATUS_EX` payload. The two differ
    /// only in two bytes after the system load (gyro cycle time against the
    /// profile counts), which this decoder skips.
    ///
    /// # Errors
    ///
    /// [`PayloadError::Truncated`] when a required field is missing.
    pub fn decode(payload: &[u8]) -> Result<Self, PayloadError> {
        let mut cursor = Cursor::new("MSP_STATUS", payload);
        let cycle_time_us = cursor.u16()?;
        cursor.take(2)?; // I2C error count
        let sensors = cursor.u16()?;
        let low_modes = cursor.u32()?;
        let pid_profile = cursor.u8()?;
        let system_load = cursor.u16()?;
        cursor.take(2)?;
        let extra_mode_bytes = usize::from(cursor.u8()? & 0x0F);
        let mut high_modes = 0u128;
        // A u128 holds 96 bits past the first 32; Betaflight defines fewer.
        for (index, byte) in cursor.take(extra_mode_bytes)?.iter().take(12).enumerate() {
            high_modes |= u128::from(*byte) << (32 + 8 * index);
        }
        cursor.take(1)?; // arming-disable flag count
        let arming_disable = ArmingDisableFlags(cursor.u32()?);
        let reboot_required = cursor.optional(Cursor::u8).is_some_and(|b| b & 1 != 0);
        Ok(Self {
            cycle_time_us,
            sensors,
            modes: ModeFlags(u128::from(low_modes) | high_modes),
            pid_profile,
            system_load,
            arming_disable,
            reboot_required,
        })
    }
}
