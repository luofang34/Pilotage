//! Betaflight SITL's simulator packets, for a build with
//! `ENABLE_GAZEBO_BRIDGE=0` (the direct, "legacy" bridge) and its virtual
//! GNSS receiver.
//!
//! In that build `sitl.c` reads each `fdm_packet` field this way:
//!
//! - gyro: roll `+x`, pitch `-y`, yaw `-z` of the packet's rates. Betaflight
//!   integrates its gyro as an FLU body rate, so the packet carries the
//!   FRD body rate `(p, q, r)` unchanged.
//! - accelerometer: the negated packet value. Betaflight expects the FLU
//!   specific force (`+g` up at rest), so the packet carries its negation.
//! - attitude: the packet quaternion is the FLU-body to NWU-world rotation,
//!   which the magnetometer model and the direct-attitude path consume.
//! - GNSS: `position_xyz` is longitude, latitude (degrees), and altitude
//!   (m); `velocity_xyz` is ENU (m/s).
//! - barometer: `pressure` in pascals.
//!
//! The ignored live test `tests/betaflight_conventions.rs` checks these
//! against a running SITL.

use crate::atmosphere::pressure_pa;
use crate::geodesy::Origin;
use crate::quad::QuadState;
use crate::rotation::{Quat, Vec3};

/// Bytes in a `fdm_packet`: eighteen little-endian `f64`s.
pub const FDM_PACKET_LEN: usize = 18 * 8;
/// Bytes in a `servo_packet`: four little-endian `f32`s.
pub const SERVO_PACKET_LEN: usize = 4 * 4;

/// The packet-time spacing between two consecutive state packets.
///
/// `sitl.c` rescales Betaflight's whole clock by packet-time over
/// arrival-time, from each pair of packets less than 20 ms apart in packet
/// time. A real-time model gains nothing from that, and on a loaded host
/// it does harm: packets that queue and arrive in a burst read as time
/// running many times too fast, which stretches every Betaflight timeout
/// (RC loss first) and the PID loop's time step. Spacing packet time by
/// 25 ms keeps every pair out of that estimate, so the factor stays at its
/// initial 1 and Betaflight runs on its own real clock.
pub const PACKET_TIME_STEP_S: f64 = 0.025;

/// Timestamps for the state packets of a real-time model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PacketClock {
    next: f64,
}

impl PacketClock {
    /// A clock for packets sent at most `rate_hz` times a second, started
    /// `wall_seconds` after the Unix epoch. Starting from the scaled wall
    /// clock keeps packet time rising across a restart of the model, which
    /// Betaflight needs: it drops a packet older than the last it used.
    #[must_use]
    pub fn new(wall_seconds: f64, rate_hz: f64) -> Self {
        Self {
            next: wall_seconds * rate_hz * PACKET_TIME_STEP_S,
        }
    }

    /// The timestamp for the next packet.
    pub fn tick(&mut self) -> f64 {
        let now = self.next;
        self.next += PACKET_TIME_STEP_S;
        now
    }
}

/// One `fdm_packet`, field for field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FdmPacket {
    /// Simulation time, s.
    pub timestamp: f64,
    /// Gyro rates as the packet carries them, rad/s.
    pub angular_velocity: [f64; 3],
    /// Accelerometer as the packet carries it, m/s².
    pub linear_acceleration: [f64; 3],
    /// Attitude quaternion as the packet carries it, `w, x, y, z`.
    pub orientation: [f64; 4],
    /// ENU velocity, m/s.
    pub velocity: [f64; 3],
    /// Longitude, latitude (degrees), altitude (m).
    pub position: [f64; 3],
    /// Static pressure, Pa.
    pub pressure: f64,
}

/// FRD body rate to the packet's gyro field.
#[must_use]
pub fn gyro_field(rates_frd: Vec3) -> [f64; 3] {
    [rates_frd.x, rates_frd.y, rates_frd.z]
}

/// FRD specific force to the packet's accelerometer field.
#[must_use]
pub fn accelerometer_field(specific_force_frd: Vec3) -> [f64; 3] {
    // FLU specific force is (x, -y, -z) of FRD; the packet negates it.
    [
        -specific_force_frd.x,
        specific_force_frd.y,
        specific_force_frd.z,
    ]
}

/// FRD-to-NED attitude to the packet's FLU-to-NWU quaternion: both frame
/// changes are a half turn about x, which conjugation applies to y and z.
#[must_use]
pub fn orientation_field(frd_to_ned: Quat) -> [f64; 4] {
    [frd_to_ned.w, frd_to_ned.x, -frd_to_ned.y, -frd_to_ned.z]
}

/// The packet quaternion back to the FRD-to-NED attitude.
#[must_use]
pub fn attitude_from_field(field: [f64; 4]) -> Quat {
    Quat {
        w: field[0],
        x: field[1],
        y: -field[2],
        z: -field[3],
    }
}

impl FdmPacket {
    /// The packet describing `state` at `timestamp`, positioned around
    /// `origin`.
    #[must_use]
    pub fn from_state(timestamp: f64, state: &QuadState, origin: &Origin) -> Self {
        let (lat, lon, alt) =
            origin.to_geodetic(state.position.x, state.position.y, state.position.z);
        let v = state.velocity;
        Self {
            timestamp,
            angular_velocity: gyro_field(state.rates),
            linear_acceleration: accelerometer_field(state.specific_force),
            orientation: orientation_field(state.attitude),
            velocity: [v.y, v.x, -v.z],
            position: [lon, lat, alt],
            pressure: pressure_pa(alt),
        }
    }

    /// The packet's wire bytes.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; FDM_PACKET_LEN] {
        let fields = std::iter::once(self.timestamp)
            .chain(self.angular_velocity)
            .chain(self.linear_acceleration)
            .chain(self.orientation)
            .chain(self.velocity)
            .chain(self.position)
            .chain(std::iter::once(self.pressure));
        let mut bytes = [0; FDM_PACKET_LEN];
        for (chunk, value) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(fields) {
            *chunk = value.to_le_bytes();
        }
        bytes
    }
}

/// Decodes a `servo_packet` into motor commands (0..1 in normal mode),
/// Betaflight motor order. `None` for a datagram of any other length.
#[must_use]
pub fn decode_servo_packet(datagram: &[u8]) -> Option<[f64; 4]> {
    let words: &[[u8; 4]; 4] = datagram.as_chunks::<4>().0.try_into().ok()?;
    if datagram.len() != SERVO_PACKET_LEN {
        return None;
    }
    Some(words.map(|word| f64::from(f32::from_le_bytes(word))))
}

#[cfg(test)]
mod tests;
