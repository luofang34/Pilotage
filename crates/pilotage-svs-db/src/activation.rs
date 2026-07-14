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
use crate::tile::{CandidatePackage, Tile};
use crate::trust::TrustRoot;
use crate::verify::{UsePolicy, VerifiedPackage, verify_package};

/// The currently active, fully verified package: its manifest and its verified
/// tiles as one unit. Minted only from a [`VerifiedPackage`] token, so an active
/// package is always one that passed verification and whose tiles are exactly
/// the verified ones — never a mix across versions.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivePackage {
    manifest: PackageManifest,
    tiles: Vec<Tile>,
}

impl ActivePackage {
    /// The active manifest.
    #[must_use]
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    /// The active tile content.
    #[must_use]
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    /// The active-database id, for output and diagnostics.
    #[must_use]
    pub fn id(&self) -> ActiveDbId {
        self.manifest.active_id()
    }

    /// Whether the package is within its effectivity/expiry window at `now`.
    #[must_use]
    pub fn is_current(&self, now: DayNumber) -> bool {
        !self.manifest.effectivity.is_before_effective(now)
            && !self.manifest.effectivity.is_after_expiry(now)
    }

    /// The availability of this package at a time and a position, failing closed
    /// on a currency lapse (not yet effective, or expired) **before** any
    /// coverage check, then on a coverage exit.
    ///
    /// # Errors
    ///
    /// [`DbUnavailable::Currency`] when outside the effectivity/expiry window,
    /// or [`DbUnavailable::Coverage`] when the position is outside coverage.
    pub fn availability(
        &self,
        now: DayNumber,
        pos: &GeodeticPosition,
    ) -> Result<(), DbUnavailable> {
        if !self.is_current(now) {
            return Err(DbUnavailable::Currency);
        }
        if self.manifest.coverage.region.contains(pos) {
            Ok(())
        } else {
            Err(DbUnavailable::Coverage)
        }
    }
}

/// The in-memory active-package store. Holds at most one active package;
/// activation replaces it atomically. A monotonic `generation` counter advances
/// on every successful activation, so a verification token minted against an
/// earlier state can be detected and refused.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PackageStore {
    active: Option<ActivePackage>,
    generation: u64,
}

impl PackageStore {
    /// An empty store with no active package.
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: None,
            generation: 0,
        }
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

    /// Verifies a candidate against the trust root, the current day, and the
    /// store's current state, minting a token stamped with the state it was
    /// verified against. The token is only installable while the store is still
    /// in that state.
    ///
    /// # Errors
    ///
    /// The [`DbError`] from verification.
    pub(crate) fn verify(
        &self,
        candidate: &CandidatePackage,
        trust: &TrustRoot,
        now: DayNumber,
        policy: UsePolicy,
    ) -> Result<VerifiedPackage, DbError> {
        verify_package(candidate, trust, now, self.active_id(), policy)
            .map(|token| token.stamp_generation(self.generation))
    }

    /// Installs an already-verified package — its manifest and verified tiles
    /// together — replacing any prior one with a single assignment (the atomic
    /// swap) and advancing the generation. The token is refused with
    /// [`DbError::StaleActivation`] if the store has changed since it was
    /// verified, so a replayed token cannot roll the active package backward;
    /// the existing active package is then left unchanged.
    ///
    /// # Errors
    ///
    /// [`DbError::StaleActivation`] when the token's expected state no longer
    /// matches the store.
    pub(crate) fn activate(&mut self, verified: VerifiedPackage) -> Result<ActiveDbId, DbError> {
        if verified.expected_generation() != self.generation
            || verified.expected_active() != self.active_id()
        {
            return Err(DbError::StaleActivation {
                expected_generation: verified.expected_generation(),
                actual_generation: self.generation,
            });
        }
        let (manifest, tiles) = verified.into_parts();
        let active = ActivePackage { manifest, tiles };
        let id = active.id();
        self.active = Some(active);
        self.generation = self.generation.wrapping_add(1);
        Ok(id)
    }

    /// Verifies a candidate against the trust root, the current day, and the
    /// active id (for rollback), then activates it — atomically, in one call, so
    /// there is no reusable token to replay. On any failure the store is left
    /// holding the prior package (or nothing) unchanged; the swap is reached
    /// only after full verification succeeds.
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
        let verified = self.verify(candidate, trust, now, policy)?;
        self.activate(verified)
    }

    /// The availability of the active database at the current day and a
    /// position: the active id on success, or the typed reason a consumer maps
    /// onto [`pilotage_geo::AvailabilityReason`]. Currency is re-checked here at
    /// use time, so a package that was current at activation but has since
    /// expired fails closed rather than being served indefinitely.
    ///
    /// # Errors
    ///
    /// [`DbUnavailable::NoPackage`] when nothing is active,
    /// [`DbUnavailable::Currency`] when the active package is outside its
    /// effectivity/expiry window, or [`DbUnavailable::Coverage`] when the
    /// position is outside coverage.
    pub fn availability(
        &self,
        now: DayNumber,
        pos: &GeodeticPosition,
    ) -> Result<ActiveDbId, DbUnavailable> {
        match &self.active {
            None => Err(DbUnavailable::NoPackage),
            Some(active) => active.availability(now, pos).map(|()| active.id()),
        }
    }
}

#[cfg(test)]
mod tests;
