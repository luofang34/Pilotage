//! MSP command identifiers, as Betaflight's `msp_protocol.h` numbers them.

/// API version: protocol version, API major, API minor.
pub const MSP_API_VERSION: u16 = 1;
/// All mode activation ranges.
pub const MSP_MODE_RANGES: u16 = 34;
/// Sets one mode activation range.
pub const MSP_SET_MODE_RANGE: u16 = 35;
/// The enabled feature mask.
pub const MSP_FEATURE_CONFIG: u16 = 36;
/// Failsafe settings, including the stage-two procedure.
pub const MSP_FAILSAFE_CONFIG: u16 = 75;
/// Cycle time, sensors, active modes, and arming-disable flags.
pub const MSP_STATUS: u16 = 101;
/// Raw accelerometer, gyro (degrees per second), and magnetometer.
pub const MSP_RAW_IMU: u16 = 102;
/// The RC channel values the flight controller uses.
pub const MSP_RC: u16 = 105;
/// The GNSS solution: fix, satellites, position, speed, course.
pub const MSP_RAW_GPS: u16 = 106;
/// Roll, pitch, and heading.
pub const MSP_ATTITUDE: u16 = 108;
/// Estimated altitude and vertical speed.
pub const MSP_ALTITUDE: u16 = 109;
/// Battery voltage, consumed charge, RSSI, and current.
pub const MSP_ANALOG: u16 = 110;
/// The permanent box identifiers in the order the status mode bits use.
pub const MSP_BOXIDS: u16 = 119;
/// Battery characteristics and state.
pub const MSP_BATTERY_STATE: u16 = 130;
/// The extended status: [`MSP_STATUS`] plus profile counts.
pub const MSP_STATUS_EX: u16 = 150;
/// RC channel values for the MSP receiver.
pub const MSP_SET_RAW_RC: u16 = 200;
/// Several MSP v1 replies in one response.
pub const MSP_MULTIPLE_MSP: u16 = 230;
/// Writes the current configuration to persistent storage.
pub const MSP_EEPROM_WRITE: u16 = 250;
