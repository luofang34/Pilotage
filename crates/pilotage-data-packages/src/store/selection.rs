use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};

use crate::{Channel, PackageError, PackageId, Product, Release, error::invalid};

use super::PackageStore;

/// A named selection retained by an application, flight, or replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Owner name, such as `active-map` or a flight record ID.
    pub name: String,
    /// Root release. Its dependencies are retained with it.
    pub root: PackageId,
    /// Whether this selection can change without first being released.
    pub pinned: bool,
}

/// Admission requirements for a new selection.
#[derive(Debug, Clone)]
pub struct SelectionPolicy {
    /// Evaluation time in Unix seconds.
    pub now: i64,
    /// Required publication channel.
    pub channel: Channel,
    /// Capabilities supplied by the active renderer.
    pub renderer_capabilities: BTreeSet<String>,
    /// Explicit permission to inspect data outside its validity interval.
    pub allow_outside_validity: bool,
}

impl PackageStore {
    /// Verify and select a complete dependency set in one database transaction.
    pub fn select_blocking(
        &mut self,
        selection: &Selection,
        policy: &SelectionPolicy,
    ) -> Result<(), PackageError> {
        PackageId::try_from(selection.name.clone())?;
        if let Some(current) = self.selection_cached_blocking(&selection.name)?
            && current.pinned
            && current != *selection
        {
            return Err(PackageError::Retained {
                id: current.root.as_str().to_owned(),
                owner: current.name,
            });
        }
        self.admit_dependencies_blocking(&selection.root, policy, &mut BTreeSet::new())?;
        self.connection
            .execute(
                "INSERT INTO selections(name, root_id, pinned) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET root_id = excluded.root_id, pinned = excluded.pinned",
                params![selection.name, selection.root.as_str(), selection.pinned],
            )
            .map_err(|source| self.database_error(source))?;
        Ok(())
    }

    /// Return a retained selection without changing it when its data expires.
    pub fn selection_cached_blocking(&self, name: &str) -> Result<Option<Selection>, PackageError> {
        let row: Option<(String, bool)> = self
            .connection
            .query_row(
                "SELECT root_id, pinned FROM selections WHERE name = ?1",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|source| self.database_error(source))?;
        row.map(|(id, pinned)| {
            Ok(Selection {
                name: name.to_owned(),
                root: PackageId::try_from(id)?,
                pinned,
            })
        })
        .transpose()
    }

    /// Release a selection when its owner closes the flight, replay, or view.
    pub fn release_selection_blocking(&mut self, name: &str) -> Result<(), PackageError> {
        self.connection
            .execute("DELETE FROM selections WHERE name = ?1", [name])
            .map_err(|source| self.database_error(source))?;
        Ok(())
    }

    fn admit_dependencies_blocking(
        &self,
        id: &PackageId,
        policy: &SelectionPolicy,
        seen: &mut BTreeSet<PackageId>,
    ) -> Result<(), PackageError> {
        if !seen.insert(id.clone()) {
            return Ok(());
        }
        let release = self.verify_installed_blocking(id)?.release;
        admit_release(&release, policy)?;
        for dependency in &release.dependencies {
            let required = self.verify_installed_blocking(dependency)?.release;
            if matches!(
                release.product,
                Product::IfrLow | Product::IfrHigh | Product::Procedures
            ) && required.product == Product::Navdata
                && release.source_set != required.source_set
            {
                return Err(invalid("navigation_source_set", id.as_str()));
            }
            self.admit_dependencies_blocking(dependency, policy, seen)?;
        }
        Ok(())
    }
}

fn admit_release(release: &Release, policy: &SelectionPolicy) -> Result<(), PackageError> {
    if release.channel != policy.channel {
        return Err(invalid("channel", release.id.as_str()));
    }
    if !release.valid_at(policy.now) && !policy.allow_outside_validity {
        return Err(invalid("release_validity", release.id.as_str()));
    }
    if !release.coverage.complete && policy.channel == Channel::Stable {
        return Err(invalid("coverage", release.id.as_str()));
    }
    for capability in &release.renderer_capabilities {
        if !policy.renderer_capabilities.contains(capability) {
            return Err(invalid("renderer_capability", capability));
        }
    }
    Ok(())
}
