//! Resolve map resources from an explicit set of installed files.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, sync::OnceCell};

use crate::{MbTiles, PmTiles};

mod error;
pub use error::ResourceError;

/// Maximum size of a style resource or decoded tile.
const MAX_RESOURCE_BYTES: u64 = 32 * 1024 * 1024;

/// The file format used by a local resource.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceFormat {
    /// A PMTiles archive with XYZ tile coordinates.
    Pmtiles,
    /// An MBTiles archive with TMS tile rows.
    Mbtiles,
    /// One file, such as a sprite image or its JSON index.
    File,
}

/// A resource name and its installed file. The caller retains the data release.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceBinding {
    /// A `pilotage://` URI. Archive requests append `/z/x/y`.
    pub uri: String,
    /// The absolute path of an installed file.
    pub path: PathBuf,
    /// The reader to use for this file.
    pub format: ResourceFormat,
}

struct Entry {
    binding: ResourceBinding,
    pmtiles: OnceCell<PmTiles>,
}

/// Local resources shared by the map and its tile workers.
pub struct ResourceSet {
    entries: BTreeMap<String, Entry>,
}

impl ResourceSet {
    /// Bind exact resource names without opening files or making network requests.
    pub fn new(bindings: Vec<ResourceBinding>) -> Result<Self, ResourceError> {
        let mut entries = BTreeMap::new();
        for binding in bindings {
            if !valid_uri(&binding.uri) || !binding.path.is_absolute() {
                return Err(ResourceError::Binding {
                    uri: binding.uri,
                    path: binding.path,
                });
            }
            let uri = binding.uri.clone();
            if entries
                .insert(
                    uri.clone(),
                    Entry {
                        binding,
                        pmtiles: OnceCell::new(),
                    },
                )
                .is_some()
            {
                return Err(ResourceError::Duplicate { uri });
            }
        }
        Ok(Self { entries })
    }

    /// Read a bound file or an archive tile. Missing tiles return `None`.
    pub async fn fetch(&self, uri: &str) -> Result<Option<Vec<u8>>, ResourceError> {
        if !valid_uri(uri) {
            return Err(ResourceError::Uri {
                uri: uri.to_owned(),
            });
        }
        if let Some(entry) = self.entries.get(uri) {
            return match entry.binding.format {
                ResourceFormat::File => read_file(&entry.binding.path).await.map(Some),
                _ => Err(ResourceError::Uri {
                    uri: uri.to_owned(),
                }),
            };
        }
        let (prefix, coordinate) = tile_request(uri)?;
        let entry = self
            .entries
            .get(prefix)
            .ok_or_else(|| ResourceError::Unknown {
                uri: uri.to_owned(),
            })?;
        let [zoom, x, y] = coordinate;
        let zoom = u8::try_from(zoom).map_err(|_| ResourceError::Uri {
            uri: uri.to_owned(),
        })?;
        let bytes = match entry.binding.format {
            ResourceFormat::Pmtiles => {
                let archive = entry
                    .pmtiles
                    .get_or_try_init(|| PmTiles::open(&entry.binding.path))
                    .await?;
                archive.tile(zoom, x, y).await?.map(|bytes| bytes.to_vec())
            }
            ResourceFormat::Mbtiles => read_mbtiles(entry.binding.path.clone(), zoom, x, y).await?,
            ResourceFormat::File => {
                return Err(ResourceError::Uri {
                    uri: uri.to_owned(),
                });
            }
        };
        if bytes
            .as_ref()
            .is_some_and(|bytes| bytes.len() as u64 > MAX_RESOURCE_BYTES)
        {
            return Err(ResourceError::Size {
                path: entry.binding.path.clone(),
                limit: MAX_RESOURCE_BYTES,
            });
        }
        Ok(bytes)
    }
}

fn valid_uri(uri: &str) -> bool {
    uri.strip_prefix("pilotage://").is_some_and(|name| {
        !name.is_empty()
            && name.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            })
    })
}

fn tile_request(uri: &str) -> Result<(&str, [u32; 3]), ResourceError> {
    let invalid = || ResourceError::Uri {
        uri: uri.to_owned(),
    };
    let mut parts = uri.rsplitn(4, '/');
    let y = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let x = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let zoom = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let prefix = parts
        .next()
        .filter(|prefix| valid_uri(prefix))
        .ok_or_else(invalid)?;
    Ok((prefix, [zoom, x, y]))
}

async fn read_file(path: &PathBuf) -> Result<Vec<u8>, ResourceError> {
    let io_error = |source| ResourceError::Io {
        path: path.clone(),
        source,
    };
    let file = tokio::fs::File::open(path).await.map_err(io_error)?;
    let size = file.metadata().await.map_err(io_error)?.len();
    if size > MAX_RESOURCE_BYTES {
        return Err(ResourceError::Size {
            path: path.clone(),
            limit: MAX_RESOURCE_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_RESOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_RESOURCE_BYTES {
        return Err(ResourceError::Size {
            path: path.clone(),
            limit: MAX_RESOURCE_BYTES,
        });
    }
    Ok(bytes)
}

async fn read_mbtiles(
    path: PathBuf,
    zoom: u8,
    x: u32,
    y: u32,
) -> Result<Option<Vec<u8>>, ResourceError> {
    let worker_path = path.clone();
    tokio::task::spawn_blocking(move || {
        MbTiles::open_blocking(&worker_path)?.tile_blocking(zoom, x, y)
    })
    .await
    .map_err(|source| ResourceError::Worker { path, source })?
    .map_err(Into::into)
}

#[cfg(test)]
mod tests;
