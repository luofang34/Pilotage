use std::collections::BTreeMap;

use pilotage_data_packages::{Catalog, Channel, PackageId};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::{DeliveryError, error::response_error};

/// Publisher trust supplied by the application, separate from downloaded metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publisher {
    /// Persistent publisher ID.
    pub id: PackageId,
    /// Absolute HTTPS catalog URL.
    pub catalog_url: String,
    /// Allowed Ed25519 keys, indexed by key name.
    pub keys: BTreeMap<String, [u8; 32]>,
    /// The publication channel this configuration can supply.
    pub channel: Channel,
}

impl Publisher {
    /// Check application configuration before a network request.
    pub fn validate(&self) -> Result<(), DeliveryError> {
        https_url(&self.catalog_url)?;
        if self.keys.is_empty() {
            return Err(response_error(
                &self.catalog_url,
                "no trusted publisher keys",
            ));
        }
        Ok(())
    }

    pub(crate) fn check_channel(&self, catalog: &Catalog) -> Result<(), DeliveryError> {
        if catalog
            .releases
            .iter()
            .any(|release| release.channel != self.channel)
        {
            return Err(response_error(
                &self.catalog_url,
                "publication channel does not match",
            ));
        }
        Ok(())
    }

    pub(crate) fn artifact_url(&self, source: &str) -> Result<Url, DeliveryError> {
        let url = https_url(&self.catalog_url)?
            .join(source)
            .map_err(|source_error| DeliveryError::ParseUrl {
                url: source.to_owned(),
                source: source_error,
            })?;
        https_url(url.as_str())
    }
}

pub(crate) fn https_url(value: &str) -> Result<Url, DeliveryError> {
    let url = Url::parse(value).map_err(|source| DeliveryError::ParseUrl {
        url: value.to_owned(),
        source,
    })?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(DeliveryError::Url {
            url: value.to_owned(),
        });
    }
    Ok(url)
}
