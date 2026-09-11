use std::{collections::BTreeSet, io::Read, path::Path, time::Duration};

use pilotage_data_packages::{
    Catalog, CatalogTrust, ContentDigest, InstalledRelease, PackageError, PackageId, PackageStore,
    SignedCatalog,
};
use reqwest::blocking::Client;

use crate::{
    DeliveryError, DownloadProgress, Publisher,
    error::{http_error, response_error},
};

/// Serial package operations for an application worker.
pub struct DataDelivery {
    store: PackageStore,
    client: Client,
}

impl DataDelivery {
    /// Open persistent storage and an HTTPS transport.
    pub fn open_blocking(root: impl AsRef<Path>) -> Result<Self, DeliveryError> {
        let client = Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|source| http_error("HTTPS client", source))?;
        Ok(Self {
            store: PackageStore::open_blocking(root)?,
            client,
        })
    }

    /// Access the serial package store for local installation and selection.
    pub fn store(&mut self) -> &mut PackageStore {
        &mut self.store
    }

    /// Fetch signed metadata and persist its verified sequence.
    pub fn refresh_blocking(
        &mut self,
        publisher: &Publisher,
        now: i64,
    ) -> Result<Catalog, DeliveryError> {
        publisher.validate()?;
        let response = self
            .client
            .get(&publisher.catalog_url)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|source| http_error(&publisher.catalog_url, source))?;
        let mut bytes = Vec::new();
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| DeliveryError::Read {
                url: publisher.catalog_url.clone(),
                source,
            })?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(response_error(
                &publisher.catalog_url,
                "catalog exceeds 8 MiB",
            ));
        }
        self.accept_catalog_blocking(publisher, &serde_json::from_slice(&bytes)?, now)
    }

    /// Accept a signed catalog from a local file or downloaded response.
    pub fn accept_catalog_blocking(
        &mut self,
        publisher: &Publisher,
        envelope: &SignedCatalog,
        now: i64,
    ) -> Result<Catalog, DeliveryError> {
        publisher.validate()?;
        let catalog = envelope.verify(&CatalogTrust {
            keys: publisher.keys.clone(),
            minimum_sequence: 0,
            now,
        })?;
        publisher.check_channel(&catalog)?;
        Ok(self.store.accept_catalog_blocking(
            &publisher.id,
            envelope,
            publisher.keys.clone(),
            now,
        )?)
    }

    /// Read cached metadata and check its signature, channel, and current expiry.
    pub fn catalog_blocking(
        &self,
        publisher: &Publisher,
        now: i64,
    ) -> Result<Catalog, DeliveryError> {
        publisher.validate()?;
        let envelope = self
            .store
            .catalog_envelope_cached_blocking(&publisher.id)?
            .ok_or_else(|| DeliveryError::NoCatalog {
                publisher: publisher.id.as_str().to_owned(),
            })?;
        let catalog = envelope.verify(&CatalogTrust {
            keys: publisher.keys.clone(),
            minimum_sequence: 0,
            now,
        })?;
        publisher.check_channel(&catalog)?;
        Ok(catalog)
    }

    /// Download exact dependencies, then install the requested release.
    ///
    /// The host supplies available disk space and cancellation state.
    /// Installation does not change an active map or flight selection.
    pub fn install_blocking(
        &mut self,
        publisher: &Publisher,
        id: &PackageId,
        now: i64,
        available_bytes: u64,
        cancelled: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(DownloadProgress),
    ) -> Result<InstalledRelease, DeliveryError> {
        let catalog = self.catalog_blocking(publisher, now)?;
        let order = catalog.installation_order(id)?;
        self.check_space_blocking(&order, available_bytes)?;
        for release in order {
            if cancelled() {
                return Err(DeliveryError::Cancelled);
            }
            let plan = self.store.plan_blocking(release, available_bytes)?;
            for download in plan.downloads {
                let url = publisher.artifact_url(&download.artifact.source)?;
                crate::transfer::download_blocking(
                    &self.client,
                    &mut self.store,
                    &download,
                    &url,
                    cancelled,
                    progress,
                )?;
            }
            if let Err(error) = self.store.finish_install_blocking(release) {
                if let PackageError::Digest { ref expected, .. } = error {
                    self.store
                        .discard_download_blocking(&ContentDigest::try_from(expected.clone())?)?;
                }
                return Err(error.into());
            }
        }
        Ok(self.store.verify_installed_blocking(id)?)
    }

    fn check_space_blocking(
        &self,
        releases: &[&pilotage_data_packages::Release],
        available: u64,
    ) -> Result<(), DeliveryError> {
        let mut seen = BTreeSet::new();
        let mut required = 0u64;
        for release in releases {
            for download in self.store.plan_blocking(release, available)?.downloads {
                if seen.insert(download.artifact.sha256) {
                    required = required.saturating_add(download.artifact.bytes - download.offset);
                }
            }
        }
        if required > available {
            return Err(PackageError::Space {
                required,
                available,
            }
            .into());
        }
        Ok(())
    }
}
