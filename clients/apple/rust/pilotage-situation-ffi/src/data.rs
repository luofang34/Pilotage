use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use pilotage_data_delivery::{DataDelivery, Publisher};
use pilotage_data_packages::{Channel, PackageId, Release, Selection, SelectionPolicy};

mod error;
mod records;

pub use error::DataError;
pub use records::{DataDownloadObserver, DataSelectionRequest, DataTransferProgress};

/// Persistent data operations on a host worker queue.
#[derive(uniffi::Object)]
pub struct AviationDataSession {
    delivery: Mutex<DataDelivery>,
    cancelled: AtomicBool,
}

#[uniffi::export]
impl AviationDataSession {
    /// Open the application support directory used for installed data.
    #[uniffi::constructor]
    pub fn open_blocking(root: String) -> Result<Arc<Self>, DataError> {
        Ok(Arc::new(Self {
            delivery: Mutex::new(DataDelivery::open_blocking(root)?),
            cancelled: AtomicBool::new(false),
        }))
    }

    /// Read installed manifests and retained selections without rehashing files.
    pub fn snapshot_cached_blocking(&self) -> Result<String, DataError> {
        let mut delivery = self.lock()?;
        let store = delivery.store();
        Ok(serde_json::to_string(&records::DataSnapshot {
            installed: store.installed_releases_cached_blocking()?,
            selections: store.selections_cached_blocking()?,
        })?)
    }

    /// Import a development release supplied by the signed application bundle.
    pub fn import_bundled_blocking(
        &self,
        manifest_json: String,
        source_directory: String,
        available_bytes: u64,
    ) -> Result<(), DataError> {
        let release: Release = serde_json::from_str(&manifest_json)?;
        if release.channel != Channel::Development {
            return Err(DataError::BundleChannel);
        }
        let mut delivery = self.lock()?;
        delivery.store().import_directory_blocking(
            &release,
            std::path::Path::new(&source_directory),
            available_bytes,
        )?;
        Ok(())
    }

    /// Check the configured publisher for signed update metadata.
    pub fn refresh_blocking(&self, publisher_json: String, now: i64) -> Result<String, DataError> {
        let publisher: Publisher = serde_json::from_str(&publisher_json)?;
        Ok(serde_json::to_string(
            &self.lock()?.refresh_blocking(&publisher, now)?,
        )?)
    }

    /// Read verified update metadata when no network request is needed.
    pub fn catalog_blocking(&self, publisher_json: String, now: i64) -> Result<String, DataError> {
        let publisher: Publisher = serde_json::from_str(&publisher_json)?;
        Ok(serde_json::to_string(
            &self.lock()?.catalog_blocking(&publisher, now)?,
        )?)
    }

    /// Download one release and its dependencies without changing active data.
    pub fn download_blocking(
        &self,
        publisher_json: String,
        release_id: String,
        now: i64,
        available_bytes: u64,
        observer: Box<dyn DataDownloadObserver>,
    ) -> Result<(), DataError> {
        let publisher: Publisher = serde_json::from_str(&publisher_json)?;
        let id = PackageId::try_from(release_id)?;
        let mut delivery = self.lock()?;
        self.cancelled.store(false, Ordering::Release);
        delivery.install_blocking(
            &publisher,
            &id,
            now,
            available_bytes,
            &|| self.cancelled.load(Ordering::Acquire),
            &mut |progress| {
                observer.progress(DataTransferProgress {
                    path: progress.path,
                    received: progress.received,
                    total: progress.total,
                })
            },
        )?;
        Ok(())
    }

    /// Cancel an active transfer and retain its completed chunks.
    pub fn cancel_download(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Verify the selected release and retain its dependency set.
    pub fn select_blocking(&self, request: DataSelectionRequest) -> Result<(), DataError> {
        self.lock()?.store().select_blocking(
            &Selection {
                name: request.name,
                root: PackageId::try_from(request.release_id)?,
                pinned: request.pinned,
            },
            &SelectionPolicy {
                now: request.now,
                channel: if request.development {
                    Channel::Development
                } else {
                    Channel::Stable
                },
                renderer_capabilities: BTreeSet::from_iter(request.renderer_capabilities),
                allow_outside_validity: request.allow_outside_validity,
            },
        )?;
        Ok(())
    }

    /// Release a map, flight, or replay selection.
    pub fn release_selection_blocking(&self, name: String) -> Result<(), DataError> {
        self.lock()?.store().release_selection_blocking(&name)?;
        Ok(())
    }

    /// Verify the files of a release before a consumer opens them.
    pub fn verify_blocking(&self, release_id: String) -> Result<String, DataError> {
        let installed = self
            .lock()?
            .store()
            .verify_installed_blocking(&PackageId::try_from(release_id)?)?;
        Ok(serde_json::to_string(&installed)?)
    }

    /// Remove an unused release and reclaim objects with no remaining owner.
    pub fn remove_blocking(&self, release_id: String) -> Result<u64, DataError> {
        let mut delivery = self.lock()?;
        delivery
            .store()
            .remove_blocking(&PackageId::try_from(release_id)?)?;
        Ok(delivery.store().collect_unreferenced_blocking()?)
    }
}

impl AviationDataSession {
    fn lock(&self) -> Result<MutexGuard<'_, DataDelivery>, DataError> {
        self.delivery
            .lock()
            .map_err(|_| DataError::SessionUnavailable)
    }
}
