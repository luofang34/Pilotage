//! One coherent poll of the flight controller, in SI units and NED/FRD
//! frames.
//!
//! Betaflight's `MSP_ATTITUDE` reports roll positive right-wing-down,
//! pitch positive NOSE DOWN, and heading clockwise from north; its
//! `MSP_RAW_IMU` gyro axes are roll, nose-down pitch, and counter-clockwise
//! yaw. The conversions below turn both into FRD/NED, the frames every
//! Pilotage adapter publishes.

use std::time::Instant;

use pilotage_msp::message::permanent_id;
use pilotage_msp::{
    Altitude, ArmingDisableFlags, Attitude, BoxIds, PayloadError, RawGps, RawImu, Status,
};

/// The fewest satellites whose fix the adapter treats as a position.
pub(crate) const MIN_SATELLITES: u8 = 6;

/// The GNSS part of a poll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GnssFix {
    /// Latitude, degrees.
    pub(crate) lat_deg: f64,
    /// Longitude, degrees.
    pub(crate) lon_deg: f64,
    /// Satellites in the solution.
    pub(crate) satellites: u8,
    /// Ground velocity north and east, m/s, from speed and course.
    pub(crate) velocity_ne: [f64; 2],
}

/// One decoded poll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FcSnapshot {
    /// When the reply arrived.
    pub(crate) received_at: Instant,
    /// Poll counter within one connection.
    pub(crate) sequence: u32,
    /// Connection counter: a reconnect is a new source epoch.
    pub(crate) epoch: u32,
    /// PID loop period, µs; zero when the loop is not running.
    pub(crate) cycle_time_us: u16,
    /// Whether the ARM box is active.
    pub(crate) armed: bool,
    /// Why Betaflight would refuse to arm now.
    pub(crate) arming_disable: ArmingDisableFlags,
    /// Roll, radians, FRD (positive right wing down).
    pub(crate) roll: f64,
    /// Pitch, radians, FRD (positive nose up).
    pub(crate) pitch: f64,
    /// Heading, radians clockwise from north, in `[0, 2π)`.
    pub(crate) heading: f64,
    /// Body rates, FRD, rad/s.
    pub(crate) rates_frd: [f64; 3],
    /// Altitude above the flight controller's reference, m.
    pub(crate) altitude_m: f64,
    /// Vertical speed, m/s, positive up.
    pub(crate) climb_rate: f64,
    /// The GNSS solution, when it is a usable 3-D fix.
    pub(crate) gnss: Option<GnssFix>,
}

/// The raw replies one poll collects.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PollReplies<'a> {
    pub(crate) status: &'a [u8],
    pub(crate) attitude: &'a [u8],
    pub(crate) imu: &'a [u8],
    pub(crate) altitude: &'a [u8],
    pub(crate) gps: &'a [u8],
}

impl FcSnapshot {
    /// Decodes one poll's replies.
    pub(crate) fn decode(
        replies: PollReplies<'_>,
        boxes: &BoxIds,
        received_at: Instant,
        sequence: u32,
        epoch: u32,
    ) -> Result<Self, PayloadError> {
        let status = Status::decode(replies.status)?;
        let attitude = Attitude::decode(replies.attitude)?;
        let imu = RawImu::decode(replies.imu)?;
        let altitude = Altitude::decode(replies.altitude)?;
        let gps = RawGps::decode(replies.gps)?;
        let (roll, pitch, heading) = frd_attitude(attitude);
        Ok(Self {
            received_at,
            sequence,
            epoch,
            cycle_time_us: status.cycle_time_us,
            armed: boxes.is_active(status.modes, permanent_id::ARM),
            arming_disable: status.arming_disable,
            roll,
            pitch,
            heading,
            rates_frd: frd_rates(imu.gyro_dps()),
            altitude_m: f64::from(altitude.altitude_cm) / 100.0,
            climb_rate: f64::from(altitude.vario_cm_s) / 100.0,
            gnss: usable_fix(&gps),
        })
    }

    /// The FRD-body to NED-world attitude quaternion, `w, x, y, z`.
    pub(crate) fn quaternion(&self) -> [f64; 4] {
        let (sr, cr) = (self.roll / 2.0).sin_cos();
        let (sp, cp) = (self.pitch / 2.0).sin_cos();
        let (sy, cy) = (self.heading / 2.0).sin_cos();
        [
            cr * cp * cy + sr * sp * sy,
            sr * cp * cy - cr * sp * sy,
            cr * sp * cy + sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
        ]
    }
}

/// Betaflight attitude to FRD roll, pitch, and heading in radians.
pub(crate) fn frd_attitude(attitude: Attitude) -> (f64, f64, f64) {
    let roll = f64::from(attitude.roll_decidegrees) / 10.0;
    // Betaflight's pitch is positive nose down; FRD pitch is nose up.
    let pitch = -f64::from(attitude.pitch_decidegrees) / 10.0;
    let heading = f64::from(attitude.heading_degrees).rem_euclid(360.0);
    (roll.to_radians(), pitch.to_radians(), heading.to_radians())
}

/// Betaflight gyro axes (roll, nose-down pitch, counter-clockwise yaw) to
/// FRD body rates.
pub(crate) fn frd_rates(gyro_dps: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = gyro_dps.map(f64::to_radians);
    [x, -y, -z]
}

fn usable_fix(gps: &RawGps) -> Option<GnssFix> {
    if !gps.fix || gps.satellites < MIN_SATELLITES {
        return None;
    }
    let speed = f64::from(gps.ground_speed_cm_s) / 100.0;
    let course = (f64::from(gps.ground_course_decidegrees) / 10.0).to_radians();
    Some(GnssFix {
        lat_deg: f64::from(gps.lat_e7) * 1e-7,
        lon_deg: f64::from(gps.lon_e7) * 1e-7,
        satellites: gps.satellites,
        velocity_ne: [speed * course.cos(), speed * course.sin()],
    })
}

#[cfg(test)]
pub(crate) mod tests;
