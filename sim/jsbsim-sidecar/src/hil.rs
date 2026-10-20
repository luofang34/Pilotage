//! PX4's MAVLink simulator protocol: the sidecar sends `HIL_SENSOR`,
//! `HIL_GPS`, `HIL_STATE_QUATERNION`, and `HEARTBEAT`, and receives
//! `HIL_ACTUATOR_CONTROLS`. Payloads are in MAVLink wire order: fields
//! sorted by size, largest first, then extensions.

mod reader;

pub use reader::FrameReader;

use pilotage_mavlink::codec::{compute_crc, encode_frame_v2_from};

use crate::sensors::{GnssSample, InertialSample};
use crate::truth::Truth;

/// HEARTBEAT message id and CRC_EXTRA.
pub const HEARTBEAT: (u32, u8) = (0, 50);
/// HIL_ACTUATOR_CONTROLS message id and CRC_EXTRA.
pub const HIL_ACTUATOR_CONTROLS: (u32, u8) = (93, 47);
/// HIL_SENSOR message id and CRC_EXTRA.
pub const HIL_SENSOR: (u32, u8) = (107, 108);
/// HIL_GPS message id and CRC_EXTRA.
pub const HIL_GPS: (u32, u8) = (113, 124);
/// HIL_STATE_QUATERNION message id and CRC_EXTRA.
pub const HIL_STATE_QUATERNION: (u32, u8) = (115, 4);

/// The simulator's MAVLink identity.
const SYSTEM_ID: u8 = 1;
/// MAV_COMP_ID_SIM.
const COMPONENT_ID: u8 = 51;

/// `HIL_SENSOR.fields_updated` bits: accel, gyro, mag, abs pressure,
/// diff pressure, pressure altitude, temperature.
pub const FIELDS_IMU: u32 = 0x003f;
/// The magnetometer bits.
pub const FIELDS_MAG: u32 = 0x01c0;
/// The barometer bits: pressure, pressure altitude, temperature. PX4
/// takes a barometer sample only when all three are set.
pub const FIELDS_BARO: u32 = 0x1a00;
/// The differential-pressure bit.
pub const FIELDS_DIFF_PRESSURE: u32 = 0x0400;
/// Standard gravity, m/s²: the unit HIL_STATE_QUATERNION's milli-g is
/// defined against (ISO 80000-3), not the local gravity of the world
/// model, which the specific force it scales already contains.
const STANDARD_GRAVITY_MPS2: f64 = 9.806_65;
/// MAV_MODE_FLAG_SAFETY_ARMED in `HIL_ACTUATOR_CONTROLS.mode`.
pub const MODE_ARMED: u8 = 0x80;

/// One `HIL_ACTUATOR_CONTROLS`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActuatorControls {
    /// PX4's clock when it sent them, µs.
    pub time_us: u64,
    /// The 16 output channels: motors in [0, 1], servos in [-1, 1].
    pub controls: [f32; 16],
    /// MAV_MODE_FLAG bits.
    pub mode: u8,
    /// Flags; bit 0 says PX4 runs lockstep.
    pub flags: u64,
}

impl ActuatorControls {
    /// Whether PX4 reports itself armed.
    pub fn armed(&self) -> bool {
        self.mode & MODE_ARMED != 0
    }

    /// Decodes a payload; MAVLink 2 trims trailing zeros, so a short
    /// payload is zero-extended.
    pub fn decode(payload: &[u8]) -> Self {
        let mut full = [0u8; 81];
        let len = payload.len().min(full.len());
        full[..len].copy_from_slice(&payload[..len]);
        let u64_at = |at: usize| {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&full[at..at + 8]);
            u64::from_le_bytes(bytes)
        };
        let mut controls = [0f32; 16];
        for (i, control) in controls.iter_mut().enumerate() {
            let at = 16 + 4 * i;
            *control = f32::from_le_bytes([full[at], full[at + 1], full[at + 2], full[at + 3]]);
        }
        Self {
            time_us: u64_at(0),
            flags: u64_at(8),
            controls,
            mode: full[80],
        }
    }
}

/// Builds frames with a running sequence number.
#[derive(Debug, Default)]
pub struct Encoder {
    sequence: u8,
}

/// A little-endian payload under construction.
#[derive(Default)]
struct Payload(Vec<u8>);

impl Payload {
    fn u64(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(mut self, v: f64) -> Self {
        self.0.extend_from_slice(&(v as f32).to_le_bytes());
        self
    }
    fn u16(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i16(mut self, v: i16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
}

/// Degrees to the 1e-7 degree integers MAVLink carries.
fn deg_e7(deg: f64) -> i32 {
    (deg * 1.0e7).round() as i32
}

/// Metres per second to centimetres per second, saturating.
fn cm_s(mps: f64) -> i16 {
    (mps * 100.0)
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

impl Encoder {
    fn frame(&mut self, (id, extra): (u32, u8), payload: Payload) -> Vec<u8> {
        let mut out = vec![0u8; 12 + payload.0.len()];
        let len = encode_frame_v2_from(
            self.sequence,
            id,
            &payload.0,
            extra,
            SYSTEM_ID,
            COMPONENT_ID,
            &mut out,
        );
        self.sequence = self.sequence.wrapping_add(1);
        out.truncate(len);
        out
    }

    /// A simulator heartbeat (MAV_TYPE_GENERIC, MAV_AUTOPILOT_INVALID).
    pub fn heartbeat(&mut self) -> Vec<u8> {
        let payload = Payload::default().u32(0).u8(0).u8(8).u8(0).u8(0).u8(3);
        self.frame(HEARTBEAT, payload)
    }

    /// `HIL_SENSOR` carrying the fields `fields_updated` names.
    pub fn hil_sensor(&mut self, sample: &InertialSample, fields_updated: u32) -> Vec<u8> {
        let [ax, ay, az] = sample.accel_frd;
        let [gx, gy, gz] = sample.gyro_frd;
        let [mx, my, mz] = sample.mag_frd_gauss;
        let payload = Payload::default()
            .u64(sample.time_us)
            .f32(ax)
            .f32(ay)
            .f32(az)
            .f32(gx)
            .f32(gy)
            .f32(gz)
            .f32(mx)
            .f32(my)
            .f32(mz)
            .f32(sample.pressure_hpa)
            .f32(sample.diff_pressure_hpa)
            .f32(sample.pressure_altitude_m)
            .f32(sample.temperature_c)
            .u32(fields_updated)
            .u8(0);
        self.frame(HIL_SENSOR, payload)
    }

    /// `HIL_GPS` with a 3D fix.
    pub fn hil_gps(&mut self, fix: &GnssSample) -> Vec<u8> {
        let [vn, ve, vd] = fix.velocity_ned;
        let ground_speed = vn.hypot(ve);
        let course_cdeg = (ve.atan2(vn).to_degrees().rem_euclid(360.0) * 100.0).round() as u16;
        let payload = Payload::default()
            .u64(fix.time_us)
            .i32(deg_e7(fix.latitude_deg))
            .i32(deg_e7(fix.longitude_deg))
            .i32((fix.altitude_m * 1000.0).round() as i32)
            .u16((fix.horizontal_accuracy_m * 100.0).round() as u16)
            .u16((fix.vertical_accuracy_m * 100.0).round() as u16)
            .u16((ground_speed * 100.0).round() as u16)
            .i16(cm_s(vn))
            .i16(cm_s(ve))
            .i16(cm_s(vd))
            .u16(course_cdeg)
            .u8(3)
            .u8(16)
            .u8(0)
            // Yaw unknown: 0 means "not provided", PX4 uses the magnetometer.
            .u16(0);
        self.frame(HIL_GPS, payload)
    }

    /// `HIL_STATE_QUATERNION`: the truth PX4 republishes as ground truth.
    pub fn hil_state_quaternion(&mut self, truth: &Truth) -> Vec<u8> {
        let q = truth.attitude.quaternion();
        let [p, qr, r] = truth.rates_frd;
        let [vn, ve, vd] = truth.velocity_ned;
        let mg = |mps2: f64| {
            (mps2 / STANDARD_GRAVITY_MPS2 * 1000.0)
                .round()
                .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
        };
        let [ax, ay, az] = truth.specific_force_frd;
        let payload = Payload::default()
            .u64(super::sensors::time_us(truth.time_s))
            .f32(q.w)
            .f32(q.x)
            .f32(q.y)
            .f32(q.z)
            .f32(p)
            .f32(qr)
            .f32(r)
            .i32(deg_e7(truth.latitude_deg))
            .i32(deg_e7(truth.longitude_deg))
            .i32((truth.altitude_m * 1000.0).round() as i32)
            .i16(cm_s(vn))
            .i16(cm_s(ve))
            .i16(cm_s(vd))
            .u16((truth.calibrated_airspeed_mps * 100.0).round() as u16)
            .u16((truth.true_airspeed_mps * 100.0).round() as u16)
            .i16(mg(ax))
            .i16(mg(ay))
            .i16(mg(az));
        self.frame(HIL_STATE_QUATERNION, payload)
    }
}

/// Verifies a received frame's CRC for a message whose CRC_EXTRA is
/// known.
pub fn crc_matches(frame_body: &[u8], extra: u8, received: u16) -> bool {
    compute_crc(frame_body, extra) == received
}

#[cfg(test)]
mod tests;
