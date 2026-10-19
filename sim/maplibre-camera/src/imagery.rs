//! One imagery source drawn from two servers: the base imagery to its
//! deepest zoom, the detail imagery past it.
//!
//! The style names a single raster source whose tiles the router resolves
//! by zoom. One source keeps one drape per terrain tile; and because a
//! detail tile and its base parent belong to the same source, a detail
//! tile the server fails to deliver is stood in for by its parent, as any
//! missing tile of a source is, instead of leaving a hole.

use maplibre::{
    coords::WorldTileCoords,
    io::{
        source_client::{HttpClient, SourceFetchError},
        source_type::RasterSource,
    },
    platform::http_client::ReqwestHttpClient,
    style::source::TileAddressingScheme,
};

use crate::args::Config;

/// The URL scheme of the routed imagery source.
pub const SCHEME: &str = "camera-imagery://";
/// The routed source's tile template.
pub const TEMPLATE: &str = "camera-imagery://{z}/{x}/{y}";

/// Fetches routed imagery tiles from their server and every other URL as
/// it is, all through one cached HTTP client.
#[derive(Clone)]
pub struct ImageryRouter {
    http: ReqwestHttpClient,
    base: RasterSource,
    detail: RasterSource,
    detail_min_zoom: u8,
}

impl ImageryRouter {
    /// A router for `config`'s imagery, caching under `cache_dir`.
    #[must_use]
    pub fn new(config: &Config, cache_dir: &str) -> Self {
        let source =
            |template: &str| RasterSource::from_template(template, TileAddressingScheme::XYZ);
        Self {
            http: ReqwestHttpClient::new(Some(cache_dir.to_owned())),
            base: source(&config.base_imagery_url),
            detail: source(&config.imagery_url),
            detail_min_zoom: config.imagery_min_zoom,
        }
    }

    /// The server URL of a routed tile URL, `None` for a URL that is not a
    /// routed tile, and `Some(None)` for a routed tile outside the world.
    #[must_use]
    pub fn upstream(&self, url: &str) -> Option<Option<String>> {
        let rest = url.strip_prefix(SCHEME)?;
        let mut parts = rest.splitn(3, '/');
        let z = parts.next()?.parse::<u8>().ok()?;
        let x = parts.next()?.parse::<i32>().ok()?;
        let y = parts.next()?.parse::<i32>().ok()?;
        let tile = WorldTileCoords::from((x, y, z.into()));
        let source = if z >= self.detail_min_zoom {
            &self.detail
        } else {
            &self.base
        };
        Some(source.format(&tile))
    }
}

#[async_trait::async_trait]
impl HttpClient for ImageryRouter {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, SourceFetchError> {
        match self.upstream(url) {
            Some(Some(upstream)) => self.http.fetch(&upstream).await,
            Some(None) => Err(SourceFetchError::not_found(url)),
            None => self.http.fetch(url).await,
        }
    }
}

#[cfg(test)]
mod tests;
