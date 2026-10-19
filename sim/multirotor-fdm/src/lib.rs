//! A rigid-body quadcopter flight-dynamics model for Betaflight SITL.
//!
//! Betaflight SITL has no physics of its own: it sends motor commands over
//! UDP and expects IMU, attitude, GNSS, and barometer data back. This
//! crate is that other half. [`quad`] integrates a quad-X airframe with
//! first-order motor lag, drag, and ground contact at a fixed step;
//! [`betaflight`] converts the state into Betaflight's `fdm_packet` and
//! decodes its `servo_packet`; [`geodesy`] and [`atmosphere`] turn the
//! local position into the virtual GNSS fix and the static pressure.
//!
//! [`supervisor`] decides when to (re)start Betaflight itself, which the
//! binary does when it is given the SITL executable.
//!
//! It is simulation-only code (ADR-0040). No host or adapter depends on
//! it; a session launcher runs its binary beside the SITL process.

pub mod atmosphere;
pub mod betaflight;
pub mod geodesy;
pub mod msp_watch;
pub mod quad;
pub mod rotation;
pub mod supervisor;

pub use betaflight::{
    FDM_PACKET_LEN, FdmPacket, PacketClock, SERVO_PACKET_LEN, decode_servo_packet,
};
pub use geodesy::Origin;
pub use quad::{QuadParams, QuadState};
pub use rotation::{Quat, Vec3};
