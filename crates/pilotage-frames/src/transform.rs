//! Checked frame transforms: construction, composition, inversion.

use crate::error::FrameError;
use crate::frame::FrameId;
use crate::rotation::Quat;
use crate::time::Epoch;

/// How far a transform rotation's norm may drift before construction
/// refuses it (matches the display stack's quaternion budget).
pub const ROTATION_NORM_TOLERANCE: f32 = 0.02;

/// A rigid transform from one frame's coordinates to another's, valid
/// at exactly its epoch.
///
/// Conventions: `rotation` maps `from`-coordinates into
/// `to`-coordinates; `translation_m` is the origin of `from` expressed
/// in `to`, meters, f64 (ECEF magnitudes lose meters in f32). A
/// transform is a *snapshot*: velocities and rates transform by
/// rotation only, with no rotating-frame kinematic terms — supplying
/// those belongs to whoever produced the transform pair, never
/// implicitly here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTransform {
    from: FrameId,
    to: FrameId,
    epoch: Epoch,
    rotation: Quat,
    translation_m: [f64; 3],
}

impl FrameTransform {
    /// Builds a transform after validating the rotation (renormalized
    /// within [`ROTATION_NORM_TOLERANCE`]) and translation finiteness.
    ///
    /// # Errors
    ///
    /// [`FrameError::InvalidTransform`] for a non-rotation or a
    /// non-finite translation component.
    pub fn new(
        from: FrameId,
        to: FrameId,
        epoch: Epoch,
        rotation: Quat,
        translation_m: [f64; 3],
    ) -> Result<Self, FrameError> {
        let rotation = rotation
            .renormalized(ROTATION_NORM_TOLERANCE)
            .map_err(|_| FrameError::InvalidTransform)?;
        if !translation_m.iter().all(|component| component.is_finite()) {
            return Err(FrameError::InvalidTransform);
        }
        Ok(Self {
            from,
            to,
            epoch,
            rotation,
            translation_m,
        })
    }

    /// The frame this transform consumes coordinates in.
    pub const fn from_frame(&self) -> FrameId {
        self.from
    }

    /// The frame this transform produces coordinates in.
    pub const fn to_frame(&self) -> FrameId {
        self.to
    }

    /// The instant this transform is valid at.
    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// The validated rotation.
    pub const fn rotation(&self) -> Quat {
        self.rotation
    }

    /// The origin of `from` expressed in `to`, meters.
    pub const fn translation_m(&self) -> [f64; 3] {
        self.translation_m
    }

    /// Composes `self` (A→B) with `next` (B→C) into A→C. The junction
    /// frame and the epoch identity (clock, scale, instant) are
    /// checked; any disagreement is a typed refusal.
    ///
    /// # Errors
    ///
    /// [`FrameError::FrameMismatch`] when `next.from() != self.to()`;
    /// [`FrameError::ClockMismatch`] / [`FrameError::TimeScaleMismatch`]
    /// / [`FrameError::EpochMismatch`] when the epochs differ.
    pub fn then(&self, next: &Self) -> Result<Self, FrameError> {
        if next.from != self.to {
            return Err(FrameError::FrameMismatch {
                expected: self.to,
                found: next.from,
            });
        }
        check_epochs(self.epoch, next.epoch)?;
        let rotation = next.rotation.compose(self.rotation);
        let moved = next.rotation.rotate(self.translation_m);
        Ok(Self {
            from: self.from,
            to: next.to,
            epoch: self.epoch,
            rotation,
            translation_m: [
                moved[0] + next.translation_m[0],
                moved[1] + next.translation_m[1],
                moved[2] + next.translation_m[2],
            ],
        })
    }

    /// The inverse transform (B→A from A→B), same epoch.
    pub fn inverse(&self) -> Self {
        let rotation = self.rotation.inverse();
        let back = rotation.rotate(self.translation_m);
        Self {
            from: self.to,
            to: self.from,
            epoch: self.epoch,
            rotation,
            translation_m: [-back[0], -back[1], -back[2]],
        }
    }
}

/// Exact epoch identity, with the mismatch dimension named.
pub(crate) fn check_epochs(a: Epoch, b: Epoch) -> Result<(), FrameError> {
    if a.clock != b.clock {
        return Err(FrameError::ClockMismatch);
    }
    if a.scale != b.scale {
        return Err(FrameError::TimeScaleMismatch);
    }
    if a.nanos != b.nanos {
        return Err(FrameError::EpochMismatch {
            expected: a,
            found: b,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
