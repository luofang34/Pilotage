//! Output provenance: binding the active-database id into emitted output and
//! checking it back.
//!
//! A [`RenderStamp`] is the provenance an output frame carries: the exact
//! active-database id it was produced against. [`PackageStore::render_stamp`]
//! mints one only when the database is actually available at the time and
//! position (currency + coverage), so an unavailable database yields no stamp
//! rather than an unattributed frame. [`PackageStore::verify_output_provenance`]
//! checks a stamp back against the currently active database, so a frame drawn
//! against a since-retired package is caught instead of trusted.

use pilotage_geo::GeodeticPosition;

use crate::activation::PackageStore;
use crate::error::DbUnavailable;
use crate::identity::{ActiveDbId, DayNumber};

/// The provenance stamp an emitted output carries: the active-database id it was
/// produced against. There is no public constructor; a stamp is minted only by
/// [`PackageStore::render_stamp`], so it always names a database that was
/// available when the output was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderStamp {
    active_db: ActiveDbId,
}

impl RenderStamp {
    /// The active-database id this output was produced against.
    #[must_use]
    pub fn active_db(&self) -> ActiveDbId {
        self.active_db
    }
}

impl PackageStore {
    /// Mints a provenance stamp for output produced at `now` and `pos`, only
    /// when the active database is available there. The stamp binds the exact
    /// active id into the output so downstream can attribute and re-check it.
    ///
    /// # Errors
    ///
    /// The [`DbUnavailable`] reason the database is not available (no package,
    /// currency lapse, or coverage exit) — in which case no output is stamped.
    pub fn render_stamp(
        &self,
        now: DayNumber,
        pos: &GeodeticPosition,
    ) -> Result<RenderStamp, DbUnavailable> {
        let active_db = self.availability(now, pos)?;
        Ok(RenderStamp { active_db })
    }

    /// Checks an output's provenance stamp against the currently active
    /// database. A stamp naming a database that is no longer active is refused,
    /// so output produced against a retired package cannot be presented as
    /// current.
    ///
    /// # Errors
    ///
    /// [`DbUnavailable::NoPackage`] when nothing is active, or
    /// [`DbUnavailable::ProvenanceMismatch`] when the stamp names a different
    /// database than the active one.
    pub fn verify_output_provenance(&self, stamp: &RenderStamp) -> Result<(), DbUnavailable> {
        match self.active_id() {
            None => Err(DbUnavailable::NoPackage),
            Some(id) if id == stamp.active_db => Ok(()),
            Some(_) => Err(DbUnavailable::ProvenanceMismatch),
        }
    }
}

#[cfg(test)]
mod tests;
