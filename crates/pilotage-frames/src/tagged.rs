//! Frame-tagged kinematic state with provenance carried through
//! transforms.

use crate::error::FrameError;
use crate::frame::FrameId;
use crate::rotation::Quat;
use crate::time::Epoch;
use crate::transform::{FrameTransform, check_epochs};

/// A value expressed in an explicit frame at an explicit epoch, with
/// caller-supplied metadata (source identity, acquisition time,
/// integrity, authorization, coherence — whatever the producer stamps)
/// that every accepted transform carries through untouched. Refusing a
/// transform never fabricates or drops metadata: the input is simply
/// returned unused via the error.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tagged<T, M> {
    /// The frame `value` is expressed in. For an attitude this is the
    /// *reference* frame: the rotation maps body coordinates into it.
    pub frame: FrameId,
    /// The instant the value is valid at.
    pub epoch: Epoch,
    /// Producer-stamped provenance, preserved verbatim by transforms.
    pub meta: M,
    /// The value.
    pub value: T,
}

/// Position, meters, in the tagged frame.
pub type Position<M> = Tagged<[f64; 3], M>;
/// Velocity, meters/second, in the tagged frame.
pub type Velocity<M> = Tagged<[f64; 3], M>;
/// Acceleration, meters/second², in the tagged frame.
pub type Acceleration<M> = Tagged<[f64; 3], M>;
/// Body angular velocity, radians/second, about body axes; the tag
/// names the reference frame the rotation is measured against.
pub type AngularVelocity<M> = Tagged<[f32; 3], M>;
/// Attitude: rotation from body coordinates into the tagged frame.
pub type Attitude<M> = Tagged<Quat, M>;

/// The vehicle-neutral pose boundary: where the vehicle is and how it
/// is oriented, each component carrying its own frame/epoch/provenance
/// tag (an aircraft may pair NED position with NED attitude; a
/// spacecraft may pair ECI position with LVLH attitude — the pairing is
/// explicit, never assumed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose<M> {
    /// Orientation of the body in its tagged reference frame.
    pub attitude: Attitude<M>,
    /// Location in its tagged frame, meters.
    pub position: Position<M>,
}

/// The vehicle-neutral twist boundary: how the pose is changing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Twist<M> {
    /// Body-axis rotation rate against its tagged reference.
    pub angular_velocity: AngularVelocity<M>,
    /// Translational rate in its tagged frame.
    pub velocity: Velocity<M>,
}

fn check_input<T, M>(tagged: &Tagged<T, M>, transform: &FrameTransform) -> Result<(), FrameError> {
    if tagged.frame != transform.from_frame() {
        return Err(FrameError::FrameMismatch {
            expected: transform.from_frame(),
            found: tagged.frame,
        });
    }
    check_epochs(tagged.epoch, transform.epoch())
}

/// Transforms a position (rotation and translation apply).
///
/// # Errors
///
/// Typed refusals for frame or epoch disagreement ([`FrameError`]).
pub fn transform_position<M: Copy>(
    position: &Position<M>,
    transform: &FrameTransform,
) -> Result<Position<M>, FrameError> {
    check_input(position, transform)?;
    let rotated = transform.rotation().rotate(position.value);
    let t = transform.translation_m();
    Ok(Tagged {
        frame: transform.to_frame(),
        epoch: position.epoch,
        meta: position.meta,
        value: [rotated[0] + t[0], rotated[1] + t[1], rotated[2] + t[2]],
    })
}

/// Transforms a free vector (velocity, acceleration): rotation only —
/// a snapshot transform carries no rotating-frame kinematic terms.
///
/// # Errors
///
/// Typed refusals for frame or epoch disagreement ([`FrameError`]).
pub fn transform_vector<M: Copy>(
    vector: &Tagged<[f64; 3], M>,
    transform: &FrameTransform,
) -> Result<Tagged<[f64; 3], M>, FrameError> {
    check_input(vector, transform)?;
    Ok(Tagged {
        frame: transform.to_frame(),
        epoch: vector.epoch,
        meta: vector.meta,
        value: transform.rotation().rotate(vector.value),
    })
}

/// Re-references an attitude: body→A composed with A→B gives body→B.
/// The same physical orientation projected through different supplied
/// transforms yields deterministic but distinct inertial, LVLH, or
/// target-relative attitudes — none of them is the canonical state,
/// which stays the quaternion itself.
///
/// # Errors
///
/// Typed refusals for frame or epoch disagreement ([`FrameError`]).
pub fn transform_attitude<M: Copy>(
    attitude: &Attitude<M>,
    transform: &FrameTransform,
) -> Result<Attitude<M>, FrameError> {
    check_input(attitude, transform)?;
    Ok(Tagged {
        frame: transform.to_frame(),
        epoch: attitude.epoch,
        meta: attitude.meta,
        value: transform.rotation().compose(attitude.value),
    })
}

#[cfg(test)]
mod tests;
