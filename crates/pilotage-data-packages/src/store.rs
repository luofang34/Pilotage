use std::{
    fs::{self, File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{PackageError, PackageId, Release, error::file_error};

mod download;
mod installation;
mod selection;

pub use download::{Download, InstallPlan};
pub use selection::{Selection, SelectionPolicy};

/// An installed release and its verified resource directory.
#[derive(Debug, Clone)]
pub struct InstalledRelease {
    /// Immutable release manifest.
    pub release: Release,
    /// Directory containing the artifact paths in the manifest.
    pub directory: PathBuf,
}

/// One writer for persistent package installation and retained selections.
pub struct PackageStore {
    root: PathBuf,
    connection: Connection,
    _writer_lock: File,
}

impl PackageStore {
    /// Open a persistent package store and create its index if required.
    pub fn open_blocking(root: impl AsRef<Path>) -> Result<Self, PackageError> {
        let root = root.as_ref().to_path_buf();
        for name in ["objects", "releases", "staging"] {
            let path = root.join(name);
            fs::create_dir_all(&path).map_err(|source| file_error(path, source))?;
        }
        let lock_path = root.join("writer.lock");
        let writer_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| file_error(&lock_path, source))?;
        writer_lock.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => PackageError::StoreBusy { path: root.clone() },
            TryLockError::Error(source) => file_error(&lock_path, source),
        })?;
        let path = root.join("catalog.sqlite");
        let connection =
            Connection::open(&path).map_err(|source| PackageError::Database { path, source })?;
        let store = Self {
            root,
            connection,
            _writer_lock: writer_lock,
        };
        store
            .connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS releases (id TEXT PRIMARY KEY, manifest TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS objects (digest TEXT PRIMARY KEY, bytes INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS release_objects (
                 release_id TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
                 path TEXT NOT NULL, digest TEXT NOT NULL REFERENCES objects(digest),
                 PRIMARY KEY(release_id, path));
             CREATE TABLE IF NOT EXISTS dependencies (
                 release_id TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
                 required_id TEXT NOT NULL REFERENCES releases(id),
                 PRIMARY KEY(release_id, required_id));
             CREATE TABLE IF NOT EXISTS selections (
                 name TEXT PRIMARY KEY, root_id TEXT NOT NULL REFERENCES releases(id),
                 pinned INTEGER NOT NULL);",
            )
            .map_err(|source| store.database_error(source))?;
        Ok(store)
    }

    /// Read an installed manifest. This does not rehash its artifacts.
    pub fn installed_cached_blocking(
        &self,
        id: &PackageId,
    ) -> Result<Option<InstalledRelease>, PackageError> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT manifest FROM releases WHERE id = ?1",
                [id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| self.database_error(source))?;
        json.map(|json| {
            let release: Release = serde_json::from_str(&json)?;
            release.validate()?;
            Ok(InstalledRelease {
                directory: self.release_path(id),
                release,
            })
        })
        .transpose()
    }

    /// List installed manifests without rehashing their artifacts.
    pub fn installed_releases_cached_blocking(
        &self,
    ) -> Result<Vec<InstalledRelease>, PackageError> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM releases ORDER BY id")
            .map_err(|source| self.database_error(source))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| self.database_error(source))?;
        let mut releases = Vec::new();
        for row in rows {
            let id = PackageId::try_from(row.map_err(|source| self.database_error(source))?)?;
            if let Some(release) = self.installed_cached_blocking(&id)? {
                releases.push(release);
            }
        }
        Ok(releases)
    }

    /// Remove a release that no selection or installed dependent retains.
    pub fn remove_blocking(&mut self, id: &PackageId) -> Result<(), PackageError> {
        let owner: Option<String> = self
            .connection
            .query_row(
                "SELECT name FROM selections WHERE root_id = ?1 UNION ALL
             SELECT release_id FROM dependencies WHERE required_id = ?1 LIMIT 1",
                [id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| self.database_error(source))?;
        if let Some(owner) = owner {
            return Err(PackageError::Retained {
                id: id.as_str().to_owned(),
                owner,
            });
        }
        self.connection
            .execute("DELETE FROM releases WHERE id = ?1", [id.as_str()])
            .map_err(|source| self.database_error(source))?;
        let directory = self.release_path(id);
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(|s| file_error(directory, s))?;
        }
        Ok(())
    }

    /// Reclaim object files that no installed release references.
    pub fn collect_unreferenced_blocking(&mut self) -> Result<u64, PackageError> {
        self.remove_unrecorded_directories_blocking()?;
        let mut removed = 0u64;
        let directory = self.root.join("objects");
        for entry in fs::read_dir(&directory).map_err(|s| file_error(&directory, s))? {
            let entry = entry.map_err(|s| file_error(&directory, s))?;
            let digest = entry.file_name().to_string_lossy().into_owned();
            let retained: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM release_objects WHERE digest = ?1)",
                    [&digest],
                    |row| row.get(0),
                )
                .map_err(|source| self.database_error(source))?;
            if !retained {
                let bytes = entry
                    .metadata()
                    .map_err(|s| file_error(entry.path(), s))?
                    .len();
                fs::remove_file(entry.path()).map_err(|s| file_error(entry.path(), s))?;
                self.connection
                    .execute("DELETE FROM objects WHERE digest = ?1", params![digest])
                    .map_err(|source| self.database_error(source))?;
                removed = removed.saturating_add(bytes);
            }
        }
        Ok(removed)
    }

    fn remove_unrecorded_directories_blocking(&self) -> Result<(), PackageError> {
        let directory = self.root.join("releases");
        for entry in fs::read_dir(&directory).map_err(|s| file_error(&directory, s))? {
            let entry = entry.map_err(|s| file_error(&directory, s))?;
            let id = entry.file_name().to_string_lossy().into_owned();
            let retained: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM releases WHERE id = ?1)",
                    [&id],
                    |row| row.get(0),
                )
                .map_err(|source| self.database_error(source))?;
            if !retained {
                fs::remove_dir_all(entry.path()).map_err(|s| file_error(entry.path(), s))?;
            }
        }
        Ok(())
    }

    fn release_path(&self, id: &PackageId) -> PathBuf {
        self.root.join("releases").join(id.as_str())
    }

    fn database_error(&self, source: rusqlite::Error) -> PackageError {
        PackageError::Database {
            path: self.root.join("catalog.sqlite"),
            source,
        }
    }
}

#[cfg(test)]
mod tests;
