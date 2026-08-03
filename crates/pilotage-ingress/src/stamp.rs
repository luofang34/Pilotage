//! Identity and acquisition stamp for one measurement group.

use crate::source::{MeasurementClock, SourceIncarnation, SourceIntegrity, SourceRole};

/// Identity and acquisition stamp for one independently advancing
/// measurement group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasurementStamp {
    /// Explicit source role; consumers gate on this, never on id ranges.
    pub role: SourceRole,
    /// Integrity classification of the delivering path.
    pub integrity: SourceIntegrity,
    /// Source identifier, stable within one vehicle and one role. Ids may
    /// collide across roles; the role disambiguates.
    pub source_id: u64,
    /// Opaque attachment/boot identity for the producing source.
    pub source_incarnation: SourceIncarnation,
    /// Source boot/attachment generation. A reset changes this value.
    pub source_epoch: u32,
    /// Wrapping group sequence, advanced only for a new measurement.
    pub sequence: u32,
    /// Acquisition time in nanoseconds in [`Self::clock`].
    pub acquired_at_ns: u64,
    /// Clock domain for [`Self::acquired_at_ns`].
    pub clock: MeasurementClock,
}

impl MeasurementStamp {
    /// Whether two stamps describe the same source attachment.
    ///
    /// Identity is the triple of role, id, and incarnation. Epoch orders
    /// within one incarnation and is meaningless across incarnations, so an
    /// attachment change invalidates any ordering established under the
    /// previous one.
    #[must_use]
    pub fn same_attachment(&self, other: &Self) -> bool {
        self.source_id == other.source_id
            && self.role == other.role
            && self.source_incarnation == other.source_incarnation
    }
}
