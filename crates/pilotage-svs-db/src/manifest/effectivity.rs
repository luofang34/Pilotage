//! Effectivity: the release, effective, and expiry day numbers that bound when
//! a package may be used.
//!
//! Currency is a pure comparison against a supplied day number, never a wall
//! clock read inside this crate: the airborne/runtime path is handed the
//! current day and decides. A package is usable only on `[effective, expiry]`;
//! outside that window it fails closed.

use crate::identity::DayNumber;

/// The day numbers that bound a package's validity, ordered
/// `release <= effective <= expiry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effectivity {
    /// The day the package was released.
    pub release: DayNumber,
    /// The first day the package may be used.
    pub effective: DayNumber,
    /// The last day the package may be used.
    pub expiry: DayNumber,
}

impl Effectivity {
    /// Whether the three dates are ordered `release <= effective <= expiry`.
    #[must_use]
    pub const fn is_ordered(&self) -> bool {
        self.release.0 <= self.effective.0 && self.effective.0 <= self.expiry.0
    }

    /// Whether `now` is before the effective day.
    #[must_use]
    pub const fn is_before_effective(&self, now: DayNumber) -> bool {
        now.0 < self.effective.0
    }

    /// Whether `now` is after the expiry day.
    #[must_use]
    pub const fn is_after_expiry(&self, now: DayNumber) -> bool {
        now.0 > self.expiry.0
    }
}
