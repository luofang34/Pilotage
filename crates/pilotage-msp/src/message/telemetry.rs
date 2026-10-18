//! Attitude, GNSS, altitude, and battery replies.

use super::PayloadError;
use super::cursor::Cursor;

/// `MSP_ATTITUDE`: Betaflight's attitude estimate in its own conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attitude {
    /// Roll, tenths of a degree, positive right wing down.
    pub roll_decidegrees: i16,
    /// Pitch, tenths of a degree, in Betaflight's sign convention.
    pub pitch_decidegrees: i16,
    /// Heading, whole degrees, 0..360 clockwise from north.
    pub heading_degrees: i16,
}

impl Attitude {
    /// Decodes an `MSP_ATTITUDE` payload.
    ///
    /// # Errors
    ///
    /// [`PayloadError::Truncated`] when the payload is short.
    pub fn decode(payload: &[u8]) -> Result<Self, PayloadError> {
        let mut cursor = Cursor::new("MSP_ATTITUDE", payload);
        Ok(Self {
            roll_decidegrees: cursor.i16()?,
            pitch_decidegrees: cursor.i16()?,
            heading_degrees: cursor.i16()?,
        })
    }
}

/// `MSP_RAW_IMU`: raw sensor values on Betaflight's body axes, after
/// board alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawImu {
    /// Accelerometer, sensor counts.
    pub acc: [i16; 3],
    /// Gyro rates, whole degrees per second: roll, pitch, yaw axes.
    pub gyro_dps: [i16; 3],
    /// Magnetometer, sensor counts.
    pub mag: [i16; 3],
}

impl RawImu {
    /// Decodes an `MSP_RAW_IMU` payload.
    ///
    /// # Errors
    ///
    /// [`PayloadError::Truncated`] when the payload is short.
    pub fn decode(payload: &[u8]) -> Result<Self, PayloadError> {
        let mut cursor = Cursor::new("MSP_RAW_IMU", payload);
        let mut triple = || -> Result<[i16; 3], PayloadError> {
            Ok([cursor.i16()?, cursor.i16()?, cursor.i16()?])
        };
        Ok(Self {
            acc: triple()?,
            gyro_dps: triple()?,
            mag: triple()?,
        })
    }
}

/// `MSP_RAW_GPS`: the GNSS solution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawGps {
    /// Whether the receiver has a fix.
    pub fix: bool,
    /// Satellites used in the solution.
    pub satellites: u8,
    /// Latitude, 1e-7 degrees.
    pub lat_e7: i32,
    /// Longitude, 1e-7 degrees.
    pub lon_e7: i32,
    /// Altitude, whole metres, clamped at zero by the firmware.
    pub alt_m: u16,
    /// Ground speed, centimetres per second.
    pub ground_speed_cm_s: u16,
    /// Course over ground, tenths of a degree clockwise from north.
    pub ground_course_decidegrees: u16,
    /// Position dilution of precision times 100, when reported.
    pub pdop: Option<u16>,
    /// Horizontal dilution of precision times 100, when reported.
    pub hdop: Option<u16>,
}

impl RawGps {
    /// Decodes an `MSP_RAW_GPS` payload.
    ///
    /// # Errors
    ///
    /// [`PayloadError::Truncated`] when a required field is missing.
    pub fn decode(payload: &[u8]) -> Result<Self, PayloadError> {
        let mut cursor = Cursor::new("MSP_RAW_GPS", payload);
        let fix = cursor.u8()? != 0;
        let satellites = cursor.u8()?;
        let lat_e7 = cursor.i32()?;
        let lon_e7 = cursor.i32()?;
        let alt_m = cursor.u16()?;
        let ground_speed_cm_s = cursor.u16()?;
        let ground_course_decidegrees = cursor.u16()?;
        let pdop = cursor.optional(Cursor::u16);
        let hdop = cursor.optional(Cursor::u16);
        Ok(Self {
            fix,
            satellites,
            lat_e7,
            lon_e7,
            alt_m,
            ground_speed_cm_s,
            ground_course_decidegrees,
            pdop,
            hdop,
        })
    }
}

/// `MSP_ALTITUDE`: the fused altitude estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Altitude {
    /// Estimated altitude above the arming point, centimetres.
    pub altitude_cm: i32,
    /// Vertical speed, centimetres per second, positive up.
    pub vario_cm_s: i16,
}

impl Altitude {
    /// Decodes an `MSP_ALTITUDE` payload.
    ///
    /// # Errors
    ///
    /// [`PayloadError::Truncated`] when the payload is short.
    pub fn decode(payload: &[u8]) -> Result<Self, PayloadError> {
        let mut cursor = Cursor::new("MSP_ALTITUDE", payload);
        Ok(Self {
            altitude_cm: cursor.i32()?,
            vario_cm_s: cursor.i16()?,
        })
    }
}

/// `MSP_BATTERY_STATE`: the battery as the flight controller measures it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryState {
    /// Detected cell count; 0 means no battery is detected.
    pub cell_count: u8,
    /// Configured capacity, milliamp-hours.
    pub capacity_mah: u16,
    /// Charge drawn, milliamp-hours.
    pub drawn_mah: u16,
    /// Current, hundredths of an ampere.
    pub current_ca: i16,
    /// Battery state: 0 ok, 1 warning, 2 critical, 3 not present, 4 init.
    pub state: u8,
    /// Voltage, hundredths of a volt.
    pub voltage_cv: u16,
}

impl BatteryState {
    /// Decodes an `MSP_BATTERY_STATE` payload.
    ///
    /// # Errors
    ///
    /// [`PayloadError::Truncated`] when the payload is short.
    pub fn decode(payload: &[u8]) -> Result<Self, PayloadError> {
        let mut cursor = Cursor::new("MSP_BATTERY_STATE", payload);
        let cell_count = cursor.u8()?;
        let capacity_mah = cursor.u16()?;
        cursor.take(1)?; // legacy voltage, tenths of a volt
        let drawn_mah = cursor.u16()?;
        let current_ca = cursor.i16()?;
        let state = cursor.u8()?;
        let voltage_cv = cursor.u16()?;
        Ok(Self {
            cell_count,
            capacity_mah,
            drawn_mah,
            current_ca,
            state,
            voltage_cv,
        })
    }

    /// Whether a battery is present and measured.
    #[must_use]
    pub fn is_present(&self) -> bool {
        self.cell_count > 0 && self.state != 3
    }
}
