//! Source licenses and how they map onto package use restrictions.
//!
//! A license is a property of the source data; the chain carries it through to
//! the emitted package's signed [`pilotage_svs_db::UseRestrictions`] so a
//! downstream operational-use check sees the most restrictive constraint any
//! contributing source imposed. Restrictions are additive: the emitted package
//! carries the union of every source's restriction bits, so one restricted
//! source restricts the whole package.

use pilotage_svs_db::UseRestrictions;

/// The license a source's data is provided under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[repr(u8)]
pub enum LicenseCode {
    /// Open data with no operational-use restriction.
    Open = 0,
    /// Data restricted to non-operational use.
    NonOperational = 1,
    /// Data restricted to training use only.
    TrainingOnly = 2,
}

impl LicenseCode {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// The use restrictions this license contributes to the emitted package.
    #[must_use]
    pub const fn restrictions(self) -> UseRestrictions {
        match self {
            Self::Open => UseRestrictions::NONE,
            Self::NonOperational => UseRestrictions::NO_OPERATIONAL_USE,
            Self::TrainingOnly => UseRestrictions::TRAINING_ONLY,
        }
    }
}
