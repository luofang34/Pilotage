//! Source identity, integrity, accuracy, age, and coherent-snapshot identity
//! for a stated position or attitude.
//!
//! # Reuse of the AV-01 `MeasurementStamp` shape
//!
//! [`SourceStamp`] carries the same identity semantics as AV-01's
//! `MeasurementStamp`, mapped field for field, plus the integrity, accuracy, and
//! coherent-snapshot identity a synthetic-vision consumer needs:
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
//! an age is only computed between two readings on the same clock and scale;
//! across clock domains the age is `None`, never a silently-inferred difference.

use pilotage_frames::{Epoch, Quat};

use crate::datum::GeodeticPosition;

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

/// A 1-sigma accuracy estimate, in millimeters, for the horizontal and vertical
/// components of a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accuracy {
    /// Horizontal 1-sigma estimate, millimeters.
    pub horizontal_mm: u32,
    /// Vertical 1-sigma estimate, millimeters.
    pub vertical_mm: u32,
}

/// Identity of a coherent multi-source snapshot: two stamps sharing a
/// non-zero [`SnapshotId`] were sampled as one coherent set. `0` means the
/// reading is not part of a declared coherent snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotId(pub u64);

impl SnapshotId {
    /// The sentinel meaning the reading is not part of a coherent snapshot.
    pub const NONE: Self = Self(0);

    /// Whether this reading belongs to a declared coherent snapshot.
    #[must_use]
    pub const fn is_declared(self) -> bool {
        self.0 != 0
    }
}

/// The full identity and quality stamp of one reading.
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
    /// Accuracy estimate.
    pub accuracy: Accuracy,
    /// Coherent-snapshot identity.
    pub snapshot: SnapshotId,
}

impl SourceStamp {
    /// Age in nanoseconds relative to `now`, or `None` when `now` is on a
    /// different clock domain or time scale than the acquisition epoch — an age
    /// across clocks would be a silently-inferred difference, which this
    /// contract forbids. Saturates at zero for a `now` earlier than acquisition.
    #[must_use]
    pub fn age_ns(&self, now: Epoch) -> Option<u64> {
        if now.clock != self.acquired_at.clock || now.scale != self.acquired_at.scale {
            return None;
        }
        Some(now.nanos.saturating_sub(self.acquired_at.nanos))
    }

    /// Whether this stamp and `other` belong to the same coherent snapshot: both
    /// declare a snapshot id and the ids match.
    #[must_use]
    pub fn coherent_with(&self, other: &Self) -> bool {
        self.snapshot.is_declared() && self.snapshot == other.snapshot
    }
}

/// A geodetic position with its full identity and quality stamp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatedPosition {
    /// The stated position (datum-explicit).
    pub position: GeodeticPosition,
    /// Identity, epoch, integrity, accuracy, and snapshot of the reading.
    pub stamp: SourceStamp,
}

/// A vehicle attitude with its full identity and quality stamp. The attitude
/// rotates body directions to the local navigation frame; the frame pairing is
/// a `pilotage-frames` concern the consumer applies, never implied here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatedAttitude {
    /// Body → local-navigation rotation.
    pub attitude: Quat,
    /// Identity, epoch, integrity, accuracy, and snapshot of the reading.
    pub stamp: SourceStamp,
}

#[cfg(test)]
mod tests;
