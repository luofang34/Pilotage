//! Typed control intents and discrete actions (CTRL-01).
//!
//! A control frame commands MEANING, not raw numbers: a velocity in a named
//! reference frame, an `Arm` action — never "axis 2 = 0.7" or "button 9". The
//! families are mutually exclusive ([`ControlIntent`] is an enum), so a
//! velocity-to-attitude transition is a distinct intent (and, per CTRL-01, a
//! distinct scope) rather than a reinterpretation of the same axis numbers.

/// The reference frame a typed control intent is expressed in. Frames are
/// negotiated per intent family (a vehicle advertises the ones it accepts), so
/// a client never sends a frame the vehicle cannot interpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceFrame {
    /// Body axes, forward-right-down: `+x` forward, `+y` right, `+z` down.
    BodyFrd,
    /// Local tangent plane, north-east-down, origin at the vehicle.
    LocalNed,
    /// The gimbal payload's own frame (pitch/yaw about its mount).
    Gimbal,
}

/// A typed discrete control action. An adapter reads `Arm`, never "button 9",
/// so a rebound control cannot change what a press means at the vehicle
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlAction {
    /// Arm the vehicle.
    Arm,
    /// Disarm the vehicle.
    Disarm,
    /// Request a flight-mode change (the target mode is decided by the scheme).
    ModeRequest,
    /// Recenter the gimbal to its stowed/neutral orientation.
    GimbalRecenter,
}

/// Linear velocity (metres per second) plus yaw rate (radians per second) in a
/// reference frame — the family the browser scheme produces today.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelocityIntent {
    /// Frame the velocity components are expressed in.
    pub frame: ReferenceFrame,
    /// Forward/`+x` velocity, m/s.
    pub vx: f32,
    /// Right/`+y` velocity, m/s.
    pub vy: f32,
    /// Down/`+z` velocity, m/s.
    pub vz: f32,
    /// Yaw rate about the frame's down axis, rad/s.
    pub yaw_rate: f32,
}

/// A held position offset (metres in `frame`) and heading (radians).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionHoldIntent {
    /// Frame the offset is expressed in.
    pub frame: ReferenceFrame,
    /// `+x` offset, m.
    pub x: f32,
    /// `+y` offset, m.
    pub y: f32,
    /// `+z` offset, m.
    pub z: f32,
    /// Target heading, rad.
    pub heading: f32,
}

/// Orientation (unit quaternion, `frame`→body) plus normalized collective
/// thrust in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttitudeThrustIntent {
    /// Frame the orientation rotates from.
    pub frame: ReferenceFrame,
    /// Quaternion scalar component.
    pub qw: f32,
    /// Quaternion `x` component.
    pub qx: f32,
    /// Quaternion `y` component.
    pub qy: f32,
    /// Quaternion `z` component.
    pub qz: f32,
    /// Normalized collective thrust in `[0, 1]`.
    pub thrust: f32,
}

/// Body angular rates (radians per second) plus normalized thrust in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyRateIntent {
    /// Roll rate, rad/s.
    pub roll_rate: f32,
    /// Pitch rate, rad/s.
    pub pitch_rate: f32,
    /// Yaw rate, rad/s.
    pub yaw_rate: f32,
    /// Normalized collective thrust in `[0, 1]`.
    pub thrust: f32,
}

/// Gimbal angular rate (radians per second) about the payload's pitch and yaw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GimbalRateIntent {
    /// Pitch rate, rad/s.
    pub pitch_rate: f32,
    /// Yaw rate, rad/s.
    pub yaw_rate: f32,
}

/// Exactly one typed control-intent family. The families are mutually
/// exclusive, so a frame commands velocity OR attitude OR body rate, never an
/// ambiguous mix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlIntent {
    /// Linear velocity + yaw rate.
    Velocity(VelocityIntent),
    /// Position hold + heading.
    PositionHold(PositionHoldIntent),
    /// Attitude + collective thrust.
    AttitudeThrust(AttitudeThrustIntent),
    /// Body rates + collective thrust.
    BodyRate(BodyRateIntent),
    /// Gimbal pitch/yaw rate.
    GimbalRate(GimbalRateIntent),
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ControlIntent, ReferenceFrame, VelocityIntent};

    #[test]
    fn a_velocity_intent_carries_its_frame_and_components() {
        let intent = ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx: 1.0,
            vy: 0.0,
            vz: -0.5,
            yaw_rate: 0.25,
        });
        let ControlIntent::Velocity(velocity) = intent else {
            panic!("expected a velocity intent");
        };
        assert_eq!(velocity.frame, ReferenceFrame::BodyFrd);
        assert_eq!(velocity.vx, 1.0);
    }
}
