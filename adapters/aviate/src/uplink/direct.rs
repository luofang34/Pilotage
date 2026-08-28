//! The simulator-only exact direct attitude and collective-force path.
//!
//! The operator direct path shapes every request with attitude-rate,
//! attitude-acceleration, thrust-rate, and thrust-acceleration limits, so a
//! requested step arrives at the flight controller as a ramp. A direct
//! controller cannot be measured through that shaping.
//!
//! This module sends one exact setpoint through the same command sender:
//! the same socket, the same frame sequence, the same boot time, and the
//! same MAVLink source identity that every other uplink frame uses. It
//! keeps no state of its own, so it cannot change what the operator path
//! does on the next frame.
//!
//! SIM / NOT FOR FLIGHT.

use std::net::SocketAddr;

use pilotage_control_feel::DemandEnvelope;
use pilotage_mavlink::codec::{AttitudeTarget, encode_attitude_setpoint};
use thiserror::Error;

use crate::adapter::AviateProfile;

use super::FlightUplink;

/// Wire length of one encoded SET_ATTITUDE_TARGET frame.
pub const ATTITUDE_SETPOINT_FRAME_BYTES: usize = 51;

/// Proof that one uplink may carry exact direct test setpoints.
///
/// Only a simulation profile can mint this value, so the exact path is
/// structurally absent on a physical vehicle rather than merely refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatorDirectAuthority {
    profile: AviateProfile,
}

impl SimulatorDirectAuthority {
    /// Mints exact direct authority for a simulation profile.
    ///
    /// Returns `None` for every other profile.
    #[must_use]
    pub const fn for_profile(profile: AviateProfile) -> Option<Self> {
        match profile {
            AviateProfile::Simulation => Some(Self { profile }),
            AviateProfile::Physical | AviateProfile::OracleOnly => None,
        }
    }

    /// The profile that authorized the exact direct path.
    #[must_use]
    pub const fn profile(&self) -> AviateProfile {
        self.profile
    }

    const fn require_simulation(self) -> Result<(), ExactDirectError> {
        match self.profile {
            AviateProfile::Simulation => Ok(()),
            AviateProfile::Physical | AviateProfile::OracleOnly => {
                Err(ExactDirectError::NotSimulated)
            }
        }
    }
}

/// One exact direct attitude and collective-force setpoint.
///
/// Every axis is absolute. The caller supplies all four values, so the
/// uplink applies no heading integration and no axis of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExactDirectSetpoint {
    /// Absolute roll setpoint in radians.
    pub roll_rad: f32,
    /// Absolute pitch setpoint in radians.
    pub pitch_rad: f32,
    /// Absolute heading setpoint in radians.
    pub yaw_rad: f32,
    /// Normalized collective force in `[0, 1]`.
    pub collective_force: f32,
}

impl ExactDirectSetpoint {
    fn validate(&self, envelope: DemandEnvelope) -> Result<(), ExactDirectError> {
        for (axis, value) in [
            ("roll", self.roll_rad),
            ("pitch", self.pitch_rad),
            ("yaw", self.yaw_rad),
            ("collective", self.collective_force),
        ] {
            if !value.is_finite() {
                return Err(ExactDirectError::NotFinite { axis });
            }
        }
        for (axis, value) in [("roll", self.roll_rad), ("pitch", self.pitch_rad)] {
            if value.abs() > envelope.direct_tilt_rad {
                return Err(ExactDirectError::OutsideTiltEnvelope {
                    axis,
                    value_rad: value,
                    limit_rad: envelope.direct_tilt_rad,
                });
            }
        }
        if self.yaw_rad.abs() > core::f32::consts::PI {
            return Err(ExactDirectError::OutsideHeadingRange {
                value_rad: self.yaw_rad,
            });
        }
        if !(0.0..=1.0).contains(&self.collective_force) {
            return Err(ExactDirectError::OutsideCollectiveRange {
                value: self.collective_force,
            });
        }
        Ok(())
    }
}

/// The exact frame that one direct setpoint put on the command link.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransmittedDirectSetpoint {
    /// The setpoint the encoder wrote into the frame.
    pub setpoint: ExactDirectSetpoint,
    /// The MAVLink frame sequence the sender used.
    pub sequence: u8,
    /// The sender's boot time in milliseconds.
    pub time_boot_ms: u32,
    /// The MAVLink system identity of the sender.
    pub system_id: u8,
    /// The MAVLink component identity of the sender.
    pub component_id: u8,
    /// The command endpoint the frame was addressed to.
    pub endpoint: SocketAddr,
    /// The exact transmitted frame bytes.
    pub frame: [u8; ATTITUDE_SETPOINT_FRAME_BYTES],
}

/// One exact direct setpoint did not reach the flight controller.
///
/// The shaped operator path reports a constrained request as a boolean
/// beside a frame that still went out. An exact step has no such outcome:
/// a request that cannot be sent unchanged is a failed command, because a
/// silently altered step would be recorded as controller response.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ExactDirectError {
    /// The authority does not name a simulation profile.
    #[error("exact direct setpoints are simulator-only")]
    NotSimulated,
    /// One setpoint value is not a finite number.
    #[error("the {axis} setpoint is not a finite number")]
    NotFinite {
        /// The axis that carries the value.
        axis: &'static str,
    },
    /// One attitude setpoint leaves the direct tilt envelope.
    #[error("the {axis} setpoint {value_rad} rad leaves the {limit_rad} rad direct tilt envelope")]
    OutsideTiltEnvelope {
        /// The axis that carries the value.
        axis: &'static str,
        /// The requested value.
        value_rad: f32,
        /// The envelope limit.
        limit_rad: f32,
    },
    /// The heading setpoint is not a wrapped absolute heading.
    #[error("the heading setpoint {value_rad} rad is outside one wrapped revolution")]
    OutsideHeadingRange {
        /// The requested value.
        value_rad: f32,
    },
    /// The collective force leaves the normalized range.
    #[error("the collective force {value} is outside the normalized range")]
    OutsideCollectiveRange {
        /// The requested value.
        value: f32,
    },
    /// The command link is inside the quiet interval after an arm command.
    #[error("the command link is inside the post-arm quiet interval")]
    QuietInterval,
    /// The setpoint stream is closed because the vehicle is not airborne.
    #[error("the setpoint stream is closed until the vehicle is airborne")]
    StreamClosed,
    /// The command sender refused the datagram.
    #[error("the command sender refused the datagram for {endpoint}")]
    SendRefused {
        /// The command endpoint that refused the frame.
        endpoint: SocketAddr,
    },
}

impl FlightUplink {
    /// Sends one exact direct setpoint without curve or temporal shaping.
    ///
    /// The transmitted setpoint reaches the requested target in this one
    /// frame. The call reads the control-feel envelope to bound the
    /// request, and it neither reads nor writes any operator control-feel
    /// state: the response curve, neutral hysteresis, apply and release
    /// dynamics, integrated heading, and captured position hold all keep
    /// the values they had before the call.
    ///
    /// # Errors
    ///
    /// Returns [`ExactDirectError`] when the authority is not a simulation
    /// authority, when a requested value would be constrained, when the
    /// command link is quiet or its setpoint stream is closed, or when the
    /// sender refused the datagram.
    pub fn send_exact_direct_setpoint(
        &mut self,
        authority: &SimulatorDirectAuthority,
        setpoint: ExactDirectSetpoint,
    ) -> Result<TransmittedDirectSetpoint, ExactDirectError> {
        authority.require_simulation()?;
        setpoint.validate(self.feel.envelope())?;
        let now = self.clock.now();
        if self.in_quiet_interval(now) {
            return Err(ExactDirectError::QuietInterval);
        }
        if !self.airborne {
            return Err(ExactDirectError::StreamClosed);
        }
        let sequence = self.seq;
        let time_boot_ms = self.time_boot_ms();
        let frame = encode_attitude_setpoint(
            sequence,
            time_boot_ms,
            AttitudeTarget {
                roll_rad: setpoint.roll_rad,
                pitch_rad: setpoint.pitch_rad,
                yaw_rad: setpoint.yaw_rad,
                thrust: setpoint.collective_force,
                system_id: self.expected_system_id,
                component_id: self.expected_component_id,
            },
        );
        let failures_before = self.send_failures;
        self.send(&frame);
        if self.send_failures != failures_before {
            return Err(ExactDirectError::SendRefused {
                endpoint: self.target,
            });
        }
        Ok(TransmittedDirectSetpoint {
            setpoint,
            sequence,
            time_boot_ms,
            system_id: self.expected_system_id,
            component_id: self.expected_component_id,
            endpoint: self.target,
            frame,
        })
    }

    /// Opens the setpoint stream without an operator climb command.
    ///
    /// A trial reaches its start state through the normal path before it
    /// enters direct mode. This entry point exists for a rig that binds the
    /// uplink directly, so the exact path's closed-stream refusal stays
    /// reachable instead of being worked around with a shaped frame.
    pub fn open_setpoint_stream(&mut self, authority: &SimulatorDirectAuthority) {
        if authority.require_simulation().is_ok() {
            self.airborne = true;
        }
    }
}
