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

/// The explicit target of a mode-request action. Targets are typed vehicle
/// meanings, never scheme-local numbers; a vehicle advertises the targets it
/// accepts, and a request without one is rejected rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModeTarget {
    /// Camera flight: velocity sticks with brake-to-hold on release.
    CameraVelocity,
    /// FPV/direct flight: attitude sticks with direct collective thrust.
    FpvDirect,
    /// Hold the current position.
    Hold,
    /// Return to the launch/home position.
    Return,
}

/// A typed discrete control action. An adapter reads `Arm`, never "button 9",
/// so a rebound control cannot change what a press means at the vehicle
/// boundary. A mode request carries its explicit typed target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlAction {
    /// Arm the vehicle.
    Arm,
    /// Disarm the vehicle.
    Disarm,
    /// Request a flight-mode change to an explicit typed target.
    ModeRequest {
        /// The mode being requested.
        target: ModeTarget,
    },
    /// Recenter the gimbal to its stowed/neutral orientation.
    GimbalRecenter,
}

/// The action kinds a scope can advertise (the capability-side view of
/// [`ControlAction`], without per-command data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionKind {
    /// Arm the vehicle.
    Arm,
    /// Disarm the vehicle.
    Disarm,
    /// Request a flight-mode change.
    ModeRequest,
    /// Recenter the gimbal.
    GimbalRecenter,
}

impl ControlAction {
    /// The capability kind this action falls under.
    #[must_use]
    pub const fn kind(&self) -> ActionKind {
        match self {
            Self::Arm => ActionKind::Arm,
            Self::Disarm => ActionKind::Disarm,
            Self::ModeRequest { .. } => ActionKind::ModeRequest,
            Self::GimbalRecenter => ActionKind::GimbalRecenter,
        }
    }
}

/// Whether a frame's action set is self-contradictory: any action kind
/// repeated (two mode requests with different targets are still one kind —
/// ambiguous, not a sequence), or `Arm` together with `Disarm`. A frame
/// carrying such a set is rejected whole, executing none of it.
#[must_use]
pub fn actions_conflict(actions: &[ControlAction]) -> bool {
    let mut arm = false;
    let mut disarm = false;
    for (index, action) in actions.iter().enumerate() {
        match action.kind() {
            ActionKind::Arm => arm = true,
            ActionKind::Disarm => disarm = true,
            _ => {}
        }
        if actions[index + 1..]
            .iter()
            .any(|later| later.kind() == action.kind())
        {
            return true;
        }
    }
    arm && disarm
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
    use super::{
        ControlAction, ControlIntent, ModeTarget, ReferenceFrame, VelocityIntent, actions_conflict,
    };

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

    #[test]
    fn conflicting_and_duplicate_action_sets_are_detected() {
        assert!(!actions_conflict(&[]));
        assert!(!actions_conflict(&[
            ControlAction::Arm,
            ControlAction::GimbalRecenter
        ]));
        assert!(
            actions_conflict(&[ControlAction::Arm, ControlAction::Disarm]),
            "arm+disarm conflicts"
        );
        assert!(
            actions_conflict(&[ControlAction::Arm, ControlAction::Arm]),
            "a repeated kind is a duplicate"
        );
        assert!(
            actions_conflict(&[
                ControlAction::ModeRequest {
                    target: ModeTarget::Hold
                },
                ControlAction::ModeRequest {
                    target: ModeTarget::Return
                },
            ]),
            "two mode requests are one repeated kind, not a sequence"
        );
    }
}
