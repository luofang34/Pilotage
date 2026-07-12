//! The explicit NED adapter for aircraft horizon presentation.
//!
//! Aircraft horizon/unusual-attitude presentation is one typed consumer
//! of canonical state, not an invariant of it. This adapter is the one
//! place that selection happens: it accepts an attitude only when its
//! reference frame is [`FrameId::Ned`] and hands the body→NED
//! quaternion to the instrument stack unchanged — so PFD output is
//! byte-identical to feeding the quaternion directly. Any other
//! reference is a typed refusal: no local-vertical assumption is ever
//! fabricated for a frame that has no horizon.

use crate::error::FrameError;
use crate::frame::FrameId;
use crate::rotation::Quat;
use crate::tagged::Attitude;

/// Extracts the body→NED attitude for horizon presentation.
///
/// # Errors
///
/// [`FrameError::FrameMismatch`] when the attitude's reference frame is
/// anything but NED — attitude in an inertial or orbit frame remains
/// meaningful, but it has no horizon until a caller supplies the
/// NED-referencing transform explicitly.
pub fn ned_attitude<M>(attitude: &Attitude<M>) -> Result<Quat, FrameError> {
    if attitude.frame != FrameId::Ned {
        return Err(FrameError::FrameMismatch {
            expected: FrameId::Ned,
            found: attitude.frame,
        });
    }
    Ok(attitude.value)
}

#[cfg(test)]
mod tests;
