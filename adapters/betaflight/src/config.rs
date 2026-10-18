//! Typed Betaflight adapter configuration and the advertised envelope.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

/// Full-envelope horizontal speed, m/s: the same as the PX4 and Aviate
/// multicopters, so the operator's stick means the same on all of them.
pub const MAX_HORIZONTAL_MPS: f32 = 3.0;
/// Full-envelope climb or sink rate, m/s.
pub const MAX_VERTICAL_MPS: f32 = 1.5;
/// Full-envelope yaw rate, rad/s.
pub const MAX_YAW_RATE_RPS: f32 = 1.5;

/// Betaflight operating profiles supported by this adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetaflightProfile {
    /// Betaflight SITL closed by a simulated airframe.
    Simulation,
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
