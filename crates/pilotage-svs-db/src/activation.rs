//! Atomic activation: a pure validate-then-swap over an in-memory store.
//!
//! Activation is two phases. [`verify_package`] validates a candidate in full
//! and returns a [`VerifiedPackage`] token without touching the store;
//! [`PackageStore::activate`] takes that token and installs it with a single
//! assignment. The store therefore only ever holds a complete, verified package
//! or the complete prior one — an interruption between the phases leaves the
//! prior package intact, and a failed verification never reaches the swap. There
//! is no observable partial or mixed-version state.
//!
//! The store is pure state: no file or network I/O lives here, so the whole
//! transition is exercised with in-memory fixtures. The airborne/runtime path
//! reads the active id and answers coverage queries; it does not download or
//! mutate the database.

use pilotage_geo::GeodeticPosition;

use crate::error::{DbError, DbUnavailable};
use crate::identity::ActiveDbId;
use crate::identity::DayNumber;
use crate::manifest::PackageManifest;
use crate::tile::CandidatePackage;
use crate::trust::TrustRoot;
use crate::verify::{UsePolicy, VerifiedPackage, verify_package};

/// The currently active, fully verified package. Minted only from a
/// [`VerifiedPackage`] token, so an active package is always one that passed
/// verification.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivePackage {
    manifest: PackageManifest,
}

impl ActivePackage {
    /// The active manifest.
    #[must_use]
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    /// The active-database id, for output and diagnostics.
    #[must_use]
    pub fn id(&self) -> ActiveDbId {
        self.manifest.active_id()
    }

    /// Whether the active package covers a position, mapping a coverage exit to
    /// the typed reason a consumer surfaces.
    ///
    /// # Errors
    ///
    /// [`DbUnavailable::Coverage`] when the position lies outside the covered
    /// region.
    pub fn availability_for_position(&self, pos: &GeodeticPosition) -> Result<(), DbUnavailable> {
        if self.manifest.coverage.region.contains(pos) {
            Ok(())
        } else {
            Err(DbUnavailable::Coverage)
        }
    }
}

/// The in-memory active-package store. Holds at most one active package;
/// activation replaces it atomically.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PackageStore {
    active: Option<ActivePackage>,
}

impl PackageStore {
    /// An empty store with no active package.
    #[must_use]
    pub fn new() -> Self {
        Self { active: None }
    }

    /// The active package, if any.
    #[must_use]
    pub fn active(&self) -> Option<&ActivePackage> {
        self.active.as_ref()
    }

    /// The active-database id, if any, to carry into rendered output and
    /// diagnostics.
    #[must_use]
    pub fn active_id(&self) -> Option<ActiveDbId> {
        self.active.as_ref().map(ActivePackage::id)
    }

    /// A one-line diagnostic naming the active database, or its absence.
    #[must_use]
    pub fn diagnostic_line(&self) -> String {
        match self.active_id() {
            Some(id) => format!("active database {id}"),
            None => "no active database".to_string(),
        }
    }

    /// Installs an already-verified package, replacing any prior one with a
    /// single assignment (the atomic swap). Returns the new active id.
    pub fn activate(&mut self, verified: VerifiedPackage) -> ActiveDbId {
        let active = ActivePackage {
            manifest: verified.into_manifest(),
        };
        let id = active.id();
        self.active = Some(active);
        id
    }

    /// Verifies a candidate against the trust root, the current day, and the
    /// active id (for rollback), then activates it. On any failure the store is
    /// left holding the prior package (or nothing) unchanged — the swap is
    /// reached only after full verification succeeds.
    ///
    /// # Errors
    ///
    /// The [`DbError`] from verification; the store is untouched in that case.
    pub fn stage_and_activate(
        &mut self,
        candidate: &CandidatePackage,
        trust: &TrustRoot,
        now: DayNumber,
        policy: UsePolicy,
    ) -> Result<ActiveDbId, DbError> {
        let verified = verify_package(candidate, trust, now, self.active_id(), policy)?;
        Ok(self.activate(verified))
    }

    /// The availability of the active database at a position: the active id on
    /// success, or the typed reason a consumer maps onto
    /// [`pilotage_geo::AvailabilityReason`] (no package, or a coverage exit).
    ///
    /// # Errors
    ///
    /// [`DbUnavailable::NoPackage`] when nothing is active, or
    /// [`DbUnavailable::Coverage`] when the position is outside coverage.
    pub fn availability_for_position(
        &self,
        pos: &GeodeticPosition,
    ) -> Result<ActiveDbId, DbUnavailable> {
        match &self.active {
            None => Err(DbUnavailable::NoPackage),
            Some(active) => active.availability_for_position(pos).map(|()| active.id()),
        }
    }
}

#[cfg(test)]
mod tests;
