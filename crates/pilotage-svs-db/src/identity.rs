//! Stable identities for a database package: the dataset and provider it comes
//! from, its version, effectivity day numbers, and the active-database id that
//! is carried into rendered output and diagnostics.
//!
//! Versions are ordered ([`PackageVersion`] is `Ord`) so an out-of-policy
//! rollback is a comparison, not a guess. Day numbers ([`DayNumber`]) are a
//! bare count of days from a fixed simulator epoch, ordered the same way, so
//! effectivity and expiry are decided without any calendar arithmetic.

use core::fmt;

/// Identity of an aeronautical dataset (the logical database this package is a
/// release of). Compared for equality; a different dataset is a different
/// database, never a rollback of this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatasetId(pub u64);

/// Identity of the data provider that produced a dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderId(pub u64);

/// A package version, ordered major-then-minor-then-patch. Ordering is the
/// rollback policy's only input: a candidate older than the active package of
/// the same dataset is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageVersion {
    /// Major version.
    pub major: u16,
    /// Minor version.
    pub minor: u16,
    /// Patch version.
    pub patch: u16,
}

impl PackageVersion {
    /// Builds a version.
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A day number: whole days since a fixed simulator epoch. Ordered, so
/// effectivity (`effective <= now`) and expiry (`now <= expiry`) are plain
/// comparisons with no calendar math and no clock domain to confuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DayNumber(pub u32);

impl fmt::Display for DayNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The identity of the currently active database, carried into rendered output
/// and diagnostics so every produced frame can name the exact package it was
/// drawn against. Identity is content-addressed: [`Self::content_hash`] is the
/// hash of the signed manifest bytes (which cover the tile-root), so two
/// different databases can never share an id even when re-signed under the same
/// dataset and version. The `simulation_only` flag travels with the id so a
/// simulator package can never be presented as an operational one downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveDbId {
    /// The dataset this package is a release of.
    pub dataset: DatasetId,
    /// The provider that produced it.
    pub provider: ProviderId,
    /// The package version.
    pub version: PackageVersion,
    /// Whether the package is a permanently-marked simulator fixture.
    pub simulation_only: bool,
    /// The content hash of the signed manifest, binding this id to the exact
    /// package content. Distinct content yields a distinct id even at the same
    /// version.
    pub content_hash: [u8; 32],
}

impl fmt::Display for ActiveDbId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "svsdb:{:016x}/{:016x}@{}#{:02x}{:02x}{:02x}{:02x}{}",
            self.dataset.0,
            self.provider.0,
            self.version,
            self.content_hash[0],
            self.content_hash[1],
            self.content_hash[2],
            self.content_hash[3],
            if self.simulation_only { " SIM" } else { "" }
        )
    }
}
