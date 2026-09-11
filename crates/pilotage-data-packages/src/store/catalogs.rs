use std::collections::BTreeMap;

use rusqlite::OptionalExtension;

use crate::{Catalog, CatalogTrust, PackageError, PackageId, SignedCatalog, error::invalid};

use super::PackageStore;

impl PackageStore {
    /// Verify a catalog and retain its sequence across application restarts.
    pub fn accept_catalog_blocking(
        &mut self,
        publisher: &PackageId,
        envelope: &SignedCatalog,
        keys: BTreeMap<String, [u8; 32]>,
        now: i64,
    ) -> Result<Catalog, PackageError> {
        let current = self.catalog_envelope_cached_blocking(publisher)?;
        let previous = current
            .as_ref()
            .map(|value| serde_json::from_str::<Catalog>(&value.payload))
            .transpose()?;
        let catalog = envelope.verify(&CatalogTrust {
            keys,
            minimum_sequence: previous.as_ref().map_or(0, |value| value.sequence),
            now,
        })?;
        if let Some(previous) = previous
            && previous.sequence == catalog.sequence
            && previous != catalog
        {
            return Err(invalid("catalog_sequence_conflict", catalog.sequence));
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|source| self.database_error(source))?;
        for release in &catalog.releases {
            let previous: Option<String> = transaction
                .query_row(
                    "SELECT manifest FROM published_releases WHERE publisher = ?1 AND id = ?2",
                    [publisher.as_str(), release.id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|source| self.database_error(source))?;
            if let Some(previous) = previous
                && serde_json::from_str::<crate::Release>(&previous)? != *release
            {
                return Err(PackageError::IdentityConflict {
                    id: release.id.as_str().to_owned(),
                });
            }
            transaction.execute(
                "INSERT OR IGNORE INTO published_releases(publisher, id, manifest) VALUES (?1, ?2, ?3)",
                [publisher.as_str(), release.id.as_str(), &serde_json::to_string(release)?],
            ).map_err(|source| self.database_error(source))?;
        }
        transaction
            .execute(
                "INSERT INTO catalogs(publisher, envelope) VALUES (?1, ?2)
                 ON CONFLICT(publisher) DO UPDATE SET envelope = excluded.envelope",
                [publisher.as_str(), &serde_json::to_string(envelope)?],
            )
            .map_err(|source| self.database_error(source))?;
        transaction
            .commit()
            .map_err(|source| self.database_error(source))?;
        Ok(catalog)
    }

    /// Read accepted metadata without a network request or freshness check.
    pub fn catalog_envelope_cached_blocking(
        &self,
        publisher: &PackageId,
    ) -> Result<Option<SignedCatalog>, PackageError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT envelope FROM catalogs WHERE publisher = ?1",
                [publisher.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| self.database_error(source))?;
        value
            .map(|value| Ok(serde_json::from_str(&value)?))
            .transpose()
    }
}

#[cfg(test)]
mod tests;
