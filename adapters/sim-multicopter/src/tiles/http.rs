//! Tiles over HTTP with a disk cache and an in-memory LRU.
//!
//! Many simulated vehicles can share one cache directory: a downloaded tile
//! goes to a temporary file first and is then renamed into place, so a
//! reader never sees a partial file. A tile that the server does not give
//! is remembered and not asked for again in this process.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::{TILE_SIZE, TileImage, TileKey, TileSource};

/// Tiles kept in memory per process.
const MEMORY_CAPACITY: usize = 192;
/// The time limit of one tile request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The identification sent to the tile server.
const USER_AGENT: &str = concat!("pilotage-sim-multicopter/", env!("CARGO_PKG_VERSION"));

/// A tile that could not be supplied.
#[derive(Debug, thiserror::Error)]
pub enum TileError {
    /// The HTTP client could not be built.
    #[error("cannot build the tile HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    /// The request failed.
    #[error("tile request {url} failed: {source}")]
    Request {
        /// The requested URL.
        url: String,
        /// The transport failure.
        #[source]
        source: reqwest::Error,
    },
    /// The server refused the request.
    #[error("tile request {url} returned HTTP {status}")]
    Status {
        /// The requested URL.
        url: String,
        /// The HTTP status code.
        status: u16,
    },
    /// The tile bytes are not an image.
    #[error("tile {key:?} does not decode: {source}")]
    Decode {
        /// The tile address.
        key: TileKey,
        /// The decoder failure.
        #[source]
        source: image::ImageError,
    },
    /// The tile is not 256 by 256 pixels.
    #[error("tile {key:?} is {width}x{height}, not {TILE_SIZE}x{TILE_SIZE}")]
    Size {
        /// The tile address.
        key: TileKey,
        /// The decoded width.
        width: u32,
        /// The decoded height.
        height: u32,
    },
    /// The disk cache could not be written.
    #[error("cannot write tile cache file {}: {source}", path.display())]
    Cache {
        /// The cache file.
        path: PathBuf,
        /// The file-system failure.
        #[source]
        source: std::io::Error,
    },
}

/// Tiles from a URL template such as
/// `https://server/tile/{z}/{y}/{x}`.
pub struct HttpTiles {
    template: String,
    cache_dir: Option<PathBuf>,
    client: Option<reqwest::blocking::Client>,
    memory: HashMap<TileKey, (Arc<TileImage>, u64)>,
    missing: HashSet<TileKey>,
    uses: u64,
    failures: u64,
}

impl HttpTiles {
    /// A source for `template`, keeping downloads in `cache_dir`.
    ///
    /// Build it on the thread that uses it: the blocking HTTP client must
    /// not live inside an async runtime.
    ///
    /// # Errors
    /// Returns [`TileError::Client`] when the HTTP client cannot be built.
    pub fn new(template: String, cache_dir: Option<PathBuf>) -> Result<Self, TileError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(TileError::Client)?;
        Ok(Self::with_client(template, cache_dir, Some(client)))
    }

    /// A source that reads only the disk cache; for tests and offline runs.
    #[must_use]
    pub fn cache_only(template: String, cache_dir: PathBuf) -> Self {
        Self::with_client(template, Some(cache_dir), None)
    }

    fn with_client(
        template: String,
        cache_dir: Option<PathBuf>,
        client: Option<reqwest::blocking::Client>,
    ) -> Self {
        Self {
            template,
            cache_dir,
            client,
            memory: HashMap::new(),
            missing: HashSet::new(),
            uses: 0,
            failures: 0,
        }
    }

    /// The URL of one tile.
    #[must_use]
    pub fn url(&self, key: TileKey) -> String {
        self.template
            .replace("{z}", &key.z.to_string())
            .replace("{x}", &key.x.to_string())
            .replace("{y}", &key.y.to_string())
    }

    fn cache_path(&self, key: TileKey) -> Option<PathBuf> {
        let dir = self.cache_dir.as_ref()?;
        Some(
            dir.join(key.z.to_string())
                .join(key.x.to_string())
                .join(format!("{}.img", key.y)),
        )
    }

    fn load(&mut self, key: TileKey) -> Result<Option<TileImage>, TileError> {
        let path = self.cache_path(key);
        if let Some(bytes) = path.as_ref().and_then(|path| std::fs::read(path).ok()) {
            return decode(key, &bytes).map(Some);
        }
        let Some(client) = self.client.as_ref() else {
            return Ok(None);
        };
        let url = self.url(key);
        let response = client
            .get(&url)
            .send()
            .map_err(|source| TileError::Request {
                url: url.clone(),
                source,
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(TileError::Status {
                url,
                status: status.as_u16(),
            });
        }
        let bytes = response
            .bytes()
            .map_err(|source| TileError::Request { url, source })?;
        let tile = decode(key, &bytes)?;
        if let Some(path) = path {
            write_atomically(&path, &bytes)?;
        }
        Ok(Some(tile))
    }

    fn remember(&mut self, key: TileKey, tile: Arc<TileImage>) {
        if self.memory.len() >= MEMORY_CAPACITY
            && let Some(oldest) = self
                .memory
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key)
        {
            self.memory.remove(&oldest);
        }
        self.memory.insert(key, (tile, self.uses));
    }
}

impl TileSource for HttpTiles {
    fn tile(&mut self, key: TileKey) -> Option<Arc<TileImage>> {
        self.uses = self.uses.wrapping_add(1);
        let uses = self.uses;
        if let Some((tile, used)) = self.memory.get_mut(&key) {
            *used = uses;
            return Some(Arc::clone(tile));
        }
        if self.missing.contains(&key) {
            return None;
        }
        match self.load(key) {
            Ok(Some(tile)) => {
                let tile = Arc::new(tile);
                self.remember(key, Arc::clone(&tile));
                Some(tile)
            }
            Ok(None) => {
                self.missing.insert(key);
                None
            }
            Err(error) => {
                self.failures = self.failures.wrapping_add(1);
                // The first failures show a broken source; later ones repeat it.
                if self.failures <= 5 || self.failures.is_power_of_two() {
                    tracing::warn!(%error, failures = self.failures, "simulated camera tile missing");
                }
                self.missing.insert(key);
                None
            }
        }
    }
}

fn decode(key: TileKey, bytes: &[u8]) -> Result<TileImage, TileError> {
    let image = image::load_from_memory(bytes)
        .map_err(|source| TileError::Decode { key, source })?
        .to_rgb8();
    let (width, height) = image.dimensions();
    if width != TILE_SIZE || height != TILE_SIZE {
        return Err(TileError::Size { key, width, height });
    }
    TileImage::from_rgb(image.into_raw()).ok_or(TileError::Size { key, width, height })
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), TileError> {
    let failed = |source| TileError::Cache {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, bytes).map_err(failed)?;
    std::fs::rename(&temporary, path).map_err(failed)
}

#[cfg(test)]
mod tests;
