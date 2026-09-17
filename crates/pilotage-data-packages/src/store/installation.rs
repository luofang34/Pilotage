use std::{
    fs::{self, File},
    io,
    path::Path,
};

use rusqlite::params;

use crate::{
    PackageError, PackageId, Release,
    error::{file_error, invalid},
};

use super::{InstalledRelease, PackageStore, download::verify_file_blocking};

mod validation;

impl PackageStore {
    /// Import files from a local release directory through the staged installer.
    pub fn import_directory_blocking(
        &mut self,
        release: &Release,
        directory: &Path,
        available_bytes: u64,
    ) -> Result<InstalledRelease, PackageError> {
        let plan = self.plan_blocking(release, available_bytes)?;
        for download in plan.downloads {
            let path = directory.join(download.artifact.path.as_str());
            verify_file_blocking(&path, &download.artifact)?;
            self.discard_download_blocking(&download.artifact.sha256)?;
            let staged = self.partial_path(&download.artifact.sha256);
            let mut source = File::open(&path).map_err(|s| file_error(&path, s))?;
            let mut target = File::create(&staged).map_err(|s| file_error(&staged, s))?;
            // Local files can restart from their source. Durability is required before
            // promotion, without a disk flush for every transfer buffer.
            io::copy(&mut source, &mut target).map_err(|s| file_error(&staged, s))?;
            target.sync_all().map_err(|s| file_error(&staged, s))?;
        }
        self.finish_install_blocking(release)
    }

    /// Verify staged content and atomically record a complete installed release.
    pub fn finish_install_blocking(
        &mut self,
        release: &Release,
    ) -> Result<InstalledRelease, PackageError> {
        release.validate()?;
        if let Some(installed) = self.installed_cached_blocking(&release.id)? {
            if installed.release != *release {
                return Err(PackageError::IdentityConflict {
                    id: release.id.as_str().to_owned(),
                });
            }
            self.verify_installed_blocking(&release.id)?;
            return Ok(installed);
        }
        for dependency in &release.dependencies {
            self.verify_installed_blocking(dependency)?;
        }
        self.promote_objects_blocking(release)?;
        self.materialize_release_blocking(release)?;
        self.record_install_blocking(release)?;
        Ok(InstalledRelease {
            release: release.clone(),
            directory: self.release_path(&release.id),
        })
    }

    /// Rehash and validate every artifact before an installed release is used.
    pub fn verify_installed_blocking(
        &self,
        id: &PackageId,
    ) -> Result<InstalledRelease, PackageError> {
        let installed =
            self.installed_cached_blocking(id)?
                .ok_or_else(|| PackageError::Missing {
                    id: id.as_str().to_owned(),
                })?;
        for artifact in &installed.release.artifacts {
            let path = installed.directory.join(artifact.path.as_str());
            verify_file_blocking(&path, artifact)?;
            validation::validate_artifact_blocking(&path, artifact, &installed.release)?;
        }
        Ok(installed)
    }

    fn promote_objects_blocking(&self, release: &Release) -> Result<(), PackageError> {
        for artifact in &release.artifacts {
            let object = self.object_path(&artifact.sha256);
            let candidate = if object.exists() {
                object.clone()
            } else {
                self.partial_path(&artifact.sha256)
            };
            verify_file_blocking(&candidate, artifact)?;
            validation::validate_artifact_blocking(&candidate, artifact, release)?;
        }
        for artifact in &release.artifacts {
            let object = self.object_path(&artifact.sha256);
            if !object.exists() {
                fs::rename(self.partial_path(&artifact.sha256), &object)
                    .map_err(|s| file_error(&object, s))?;
            }
        }
        sync_directory_blocking(&self.root.join("objects"))
    }

    fn materialize_release_blocking(&self, release: &Release) -> Result<(), PackageError> {
        let destination = self.release_path(&release.id);
        if destination.exists() {
            for artifact in &release.artifacts {
                verify_file_blocking(&destination.join(artifact.path.as_str()), artifact)?;
            }
            return Ok(());
        }
        let staged = self.root.join("staging").join(release.id.as_str());
        if staged.exists() {
            fs::remove_dir_all(&staged).map_err(|s| file_error(&staged, s))?;
        }
        fs::create_dir_all(&staged).map_err(|s| file_error(&staged, s))?;
        for artifact in &release.artifacts {
            let target = staged.join(artifact.path.as_str());
            let parent = target
                .parent()
                .ok_or_else(|| invalid("artifact_path", artifact.path.as_str()))?;
            fs::create_dir_all(parent).map_err(|s| file_error(parent, s))?;
            fs::hard_link(self.object_path(&artifact.sha256), &target)
                .map_err(|s| file_error(&target, s))?;
            for directory in parent
                .ancestors()
                .take_while(|path| path.starts_with(&staged))
            {
                sync_directory_blocking(directory)?;
            }
        }
        sync_directory_blocking(&staged)?;
        fs::rename(&staged, &destination).map_err(|s| file_error(&destination, s))?;
        sync_directory_blocking(&self.root.join("releases"))
    }

    fn record_install_blocking(&mut self, release: &Release) -> Result<(), PackageError> {
        let json = serde_json::to_string(release)?;
        let path = self.root.join("catalog.sqlite");
        let database_error = |source| PackageError::Database {
            path: path.clone(),
            source,
        };
        let transaction = self.connection.transaction().map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO releases(id, manifest) VALUES (?1, ?2)",
                params![release.id.as_str(), json],
            )
            .map_err(database_error)?;
        for artifact in &release.artifacts {
            let bytes = i64::try_from(artifact.bytes)
                .map_err(|_| invalid("artifact_bytes", artifact.bytes))?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO objects(digest, bytes) VALUES (?1, ?2)",
                    params![artifact.sha256.as_str(), bytes],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "INSERT INTO release_objects(release_id, path, digest) VALUES (?1, ?2, ?3)",
                    params![
                        release.id.as_str(),
                        artifact.path.as_str(),
                        artifact.sha256.as_str()
                    ],
                )
                .map_err(database_error)?;
        }
        for dependency in &release.dependencies {
            transaction
                .execute(
                    "INSERT INTO dependencies(release_id, required_id) VALUES (?1, ?2)",
                    params![release.id.as_str(), dependency.as_str()],
                )
                .map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)
    }
}

fn sync_directory_blocking(path: &Path) -> Result<(), PackageError> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|s| file_error(path, s))
}
