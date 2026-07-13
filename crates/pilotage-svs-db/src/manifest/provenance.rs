//! Provenance: which dataset and provider a package comes from, its version,
//! the processing lineage that produced it, and the use restrictions that
//! travel with it.
//!
//! The processing chain is an ordered lineage of operations from the source
//! data to this package — a simulator stand-in for the recorded data process a
//! quality standard such as DO-200C requires. It is authenticated like the rest
//! of the manifest: it is part of the signed canonical bytes, so a package
//! cannot silently drop or rewrite its own history.

use crate::identity::{DatasetId, PackageVersion, ProviderId};

/// One step of the processing lineage: an operation code and the identity of the
/// tool that performed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessingStep {
    /// The operation performed (provider-defined code).
    pub code: u16,
    /// Identity of the tool that performed it.
    pub tool_id: u32,
}

/// The ordered processing lineage from source data to this package. Order is
/// significant and preserved in the canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingChain(pub Vec<ProcessingStep>);

impl ProcessingChain {
    /// The steps, source-first.
    #[must_use]
    pub fn steps(&self) -> &[ProcessingStep] {
        &self.0
    }

    /// Whether the lineage is empty — a package with no recorded processing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Restrictions on how a package may be used, as a bitset of restriction codes.
/// Restrictions are additive: a set bit is a constraint that applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UseRestrictions(pub u32);

impl UseRestrictions {
    /// No declared restrictions.
    pub const NONE: Self = Self(0);
    /// The package must not be used for operational purposes.
    pub const NO_OPERATIONAL_USE: Self = Self(1 << 0);
    /// The package is limited to training use.
    pub const TRAINING_ONLY: Self = Self(1 << 1);

    /// Whether every restriction bit in `other` is set here.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether these restrictions forbid operational use. Both
    /// [`Self::NO_OPERATIONAL_USE`] and [`Self::TRAINING_ONLY`] confine a
    /// package to non-operational use.
    #[must_use]
    pub const fn forbids_operational(self) -> bool {
        self.contains(Self::NO_OPERATIONAL_USE) || self.contains(Self::TRAINING_ONLY)
    }

    /// The raw bits, for the canonical serialization.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Where a package comes from and under what constraints it may be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The dataset this package is a release of.
    pub dataset: DatasetId,
    /// The provider that produced it.
    pub provider: ProviderId,
    /// The package version.
    pub version: PackageVersion,
    /// The processing lineage.
    pub processing: ProcessingChain,
    /// Restrictions on use.
    pub restrictions: UseRestrictions,
}
