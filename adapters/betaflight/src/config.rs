//! Typed Betaflight adapter configuration and the advertised envelope.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

/// Full-envelope horizontal speed, m/s: the same as the PX4 and Aviate
/// multicopters, so the operator's stick means the same on all of them.
pub const MAX_HORIZONTAL_MPS: f32 = 3.0;
/// Full-envelope climb or sink rate, m/s.
pub const MAX_VERTICAL_MPS: f32 = 1.5;
/// Full-envelope yaw rate, rad/s.
pub const MAX_YAW_RATE_RPS: f32 = 1.5;

/// Full-envelope body rate on every axis in direct flight, rad/s: the
/// 670 deg/s Betaflight's rate curve gives at full stick in
/// `sim/betaflight/sitl.cli`.
pub const MAX_BODY_RATE_RPS: f32 = 11.693_706;
/// The largest tilt an attitude demand may ask for, rad: Betaflight's
/// `angle_limit` of 30 deg.
pub const MAX_TILT_RAD: f32 = core::f32::consts::FRAC_PI_6;

/// Betaflight operating profiles supported by this adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetaflightProfile {
    /// Betaflight SITL closed by a simulated airframe. The adapter writes
    /// the aux mode ranges its channel layout needs (`link/setup.rs`).
    Simulation,
    /// A real flight controller: the adapter never writes its
    /// configuration. It reads the aux mode ranges and refuses to arm
    /// while one differs.
    Physical,
}

/// Where and how to reach the flight controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BetaflightConfig {
    profile: BetaflightProfile,
    /// The MSP stream: Betaflight SITL serves UART1 on TCP 5761.
    pub msp_endpoint: SocketAddr,
}

impl BetaflightConfig {
    /// The default endpoint for an explicit profile.
    #[must_use]
    pub fn new(profile: BetaflightProfile) -> Self {
        Self {
            profile,
            msp_endpoint: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5_761)),
        }
    }

    /// The selected profile.
    #[must_use]
    pub fn profile(&self) -> BetaflightProfile {
        self.profile
    }
}
