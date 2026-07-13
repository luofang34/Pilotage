//! Source identity, integrity, coherent-snapshot identity, and the
//! function-specific accuracy of a stated position or attitude.
//!
//! Identity and quality are separate concerns. [`SourceStamp`] carries only
//! identity, integrity, and coherent-snapshot membership — never a
//! domain-specific accuracy. Position accuracy is a length ([`PositionQuality`],
//! millimeters); attitude accuracy is an angle ([`AttitudeQuality`],
//! milliradians). A position's accuracy can never be read as an attitude's, and
//! vice versa, because they are different types.
//!
//! # Reuse of the AV-01 `MeasurementStamp` shape
//!
//! [`SourceStamp`] carries the same identity semantics as AV-01's
//! `MeasurementStamp`, mapped field for field:
//!
//! | [`SourceStamp`] | AV-01 `MeasurementStamp` |
//! |---|---|
//! | `source_id` | `source_id` |
//! | `incarnation` | `source_incarnation` |
//! | `generation` | `source_epoch` (boot/attachment generation) |
//! | `sequence` | `sequence` |
//! | `acquired_at` ([`Epoch`]) | `acquired_at_ns` + `clock`, enriched with a time scale |
//!
//! The acquisition time is an [`Epoch`] (clock **and** scale **and** nanos), so
//! an age is only a physical duration between two readings on the same clock and
//! scale; across clock domains or scales the age is a typed refusal, never a
//! silently-inferred difference, and a future sample is a typed refusal, never a
//! saturated zero.

use pilotage_frames::{Epoch, Quat};

use crate::datum::GeodeticPosition;
use crate::error::AgeError;

/// An opaque 128-bit source attachment/boot identity, compared only for
/// equality (a new incarnation is authorized at a lifecycle boundary, never
/// ordered against an old one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceIncarnation(pub [u8; 16]);

/// The trust an integrity monitor places in a reading. `Unknown` is the
/// fail-closed default: an unmonitored reading is not trusted for a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IntegrityLevel {
    /// No integrity information; treat as untrusted.
    Unknown = 0,
    /// Monitored and found untrustworthy.
    Untrusted = 1,
    /// Monitored, within bounds, but not to the highest assurance.
    Monitored = 2,
    /// Monitored and trusted to the stated accuracy bound.
    Trusted = 3,
}

impl IntegrityLevel {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes the wire byte, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Unknown),
            1 => Some(Self::Untrusted),
            2 => Some(Self::Monitored),
            3 => Some(Self::Trusted),
            _ => None,
        }
    }

    /// Whether a reading at this level may contribute to a normal scene.
    #[must_use]
    pub const fn is_trusted(self) -> bool {
        matches!(self, Self::Trusted)
    }
}

/// A 1-sigma accuracy estimate for a position, in millimeters — a length. It is
/// a distinct type from [`AttitudeQuality`] so a position's accuracy can never
/// be read as an attitude's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionQuality {
    /// Horizontal 1-sigma estimate, millimeters.
    pub horizontal_mm: u32,
    /// Vertical 1-sigma estimate, millimeters.
    pub vertical_mm: u32,
}

/// A 1-sigma accuracy estimate for an attitude, in milliradians — an angle. It
/// is a distinct type from [`PositionQuality`]: an attitude accuracy is never
/// measured in millimeters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttitudeQuality {
    /// 1-sigma angular estimate, milliradians.
    pub angular_mrad: u32,
}

/// Identity of a coherent multi-source snapshot: the coherence producer that
/// assembled it (by incarnation and generation) plus the snapshot instance id.
/// Two readings are coherent only when this full identity matches — an equal
/// numeric `id` from a different producer or generation is a different snapshot,
/// never coherent. `id == 0` means the reading is not part of a declared
/// coherent snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoherentSnapshot {
    /// The coherence producer's attachment/boot identity.
    pub producer: SourceIncarnation,
    /// The coherence producer's generation.
    pub generation: u32,
    /// The snapshot instance id within that producer/generation; `0` = none.
    pub id: u64,
}

impl CoherentSnapshot {
    /// The sentinel meaning the reading is not part of a coherent snapshot.
    pub const NONE: Self = Self {
        producer: SourceIncarnation([0; 16]),
        generation: 0,
        id: 0,
    };

    /// Whether this reading belongs to a declared coherent snapshot.
    #[must_use]
    pub const fn is_declared(self) -> bool {
        self.id != 0
    }
}

/// The identity stamp of one reading: who produced it, when, how trusted, and
/// which coherent snapshot it belongs to. It carries no domain-specific
/// accuracy — position and attitude carry their own typed quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceStamp {
    /// Adapter-defined source identity, stable within one vehicle.
    pub source_id: u64,
    /// Opaque attachment/boot identity.
    pub incarnation: SourceIncarnation,
    /// Source boot/attachment generation (AV-01 `source_epoch`).
    pub generation: u32,
    /// Wrapping group sequence.
    pub sequence: u32,
    /// Acquisition epoch: clock domain, time scale, and nanoseconds.
    pub acquired_at: Epoch,
    /// Integrity trust level.
    pub integrity: IntegrityLevel,
    /// Coherent-snapshot identity.
    pub snapshot: CoherentSnapshot,
}

impl SourceStamp {
    /// Age in nanoseconds relative to `now`, failing closed rather than
    /// inferring: a `now` on a different clock domain or time scale, or a
    /// sample acquired after `now`, is a typed [`AgeError`], never a saturated
    /// zero.
    ///
    /// # Errors
    ///
    /// [`AgeError::ClockMismatch`], [`AgeError::ScaleMismatch`], or
    /// [`AgeError::FutureSample`].
    pub fn age_ns(&self, now: Epoch) -> Result<u64, AgeError> {
        if now.clock != self.acquired_at.clock {
            return Err(AgeError::ClockMismatch);
        }
        if now.scale != self.acquired_at.scale {
            return Err(AgeError::ScaleMismatch);
        }
        if self.acquired_at.nanos > now.nanos {
            return Err(AgeError::FutureSample {
                acquired_nanos: self.acquired_at.nanos,
                now_nanos: now.nanos,
            });
        }
        Ok(now.nanos - self.acquired_at.nanos)
    }

    /// Whether this stamp and `other` belong to the same coherent snapshot.
    /// Coherence binds the full snapshot identity (producer incarnation,
    /// generation, and instance id) **and** the sampling time base (clock and
    /// scale): two readings with the same numeric snapshot id from different
    /// producers, generations, or time bases are not coherent.
    #[must_use]
    pub fn coherent_with(&self, other: &Self) -> bool {
        self.snapshot.is_declared()
            && self.snapshot == other.snapshot
            && self.acquired_at.clock == other.acquired_at.clock
            && self.acquired_at.scale == other.acquired_at.scale
    }
}

/// A geodetic position with its identity stamp and position accuracy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatedPosition {
    /// The stated position (datum-explicit).
    pub position: GeodeticPosition,
    /// Identity, epoch, integrity, and snapshot of the reading.
    pub stamp: SourceStamp,
    /// Position accuracy (a length).
    pub quality: PositionQuality,
}

/// A vehicle attitude with its identity stamp and angular accuracy. The attitude
/// rotates body directions to the local navigation frame; the frame pairing is
/// a `pilotage-frames` concern the consumer applies, never implied here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatedAttitude {
    /// Body → local-navigation rotation.
    pub attitude: Quat,
    /// Identity, epoch, integrity, and snapshot of the reading.
    pub stamp: SourceStamp,
    /// Attitude accuracy (an angle).
    pub quality: AttitudeQuality,
}

#[cfg(test)]
mod tests;
