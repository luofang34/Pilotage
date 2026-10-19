//! Fills the tile cache around the session's site before the cameras start.
//!
//! The default imagery is a dynamic image server: a cold tile takes seconds
//! to render and the server answers some requests with a 5xx. Fetching the
//! site's tiles first, through the same cached HTTP client the maps use,
//! means the cameras open on imagery instead of on bare ground. The area is
//! bounded: a fixed square of tiles around the site at each zoom, so deeper
//! zooms cover less ground.

use std::sync::Arc;
use std::time::{Duration, Instant};

use maplibre::{
    coords::WorldTileCoords,
    io::{source_client::HttpClient, source_type::RasterSource},
    platform::http_client::ReqwestHttpClient,
    style::source::TileAddressingScheme,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{info, warn};

use crate::args::Config;

/// Tiles fetched on each side of the site's tile at every zoom.
const RADIUS_TILES: i32 = 4;
/// Base imagery zooms fetched, ending at the base's deepest.
const BASE_LEVELS: u8 = 5;
/// Terrain zooms fetched, ending at the terrain source's deepest (15).
const DEM_ZOOMS: std::ops::RangeInclusive<u8> = 10..=15;
/// Requests in flight at once, kept low for a public server.
const CONCURRENCY: usize = 8;
/// Attempts per tile; a server error or a dropped connection is retried
/// after a doubling delay.
const ATTEMPTS: u32 = 4;
/// Delay before the first retry.
const FIRST_BACKOFF: Duration = Duration::from_millis(250);
/// The longest a pre-warm holds the cameras back; tiles still missing then
/// load as the cameras ask for them.
const DEADLINE: Duration = Duration::from_secs(240);

/// What a pre-warm did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Prewarmed {
    /// Tiles now in the cache.
    pub fetched: usize,
    /// Tiles the server states it does not have.
    pub absent: usize,
    /// Tiles that failed every attempt.
    pub failed: usize,
    /// Tiles not finished by the deadline.
    pub unfinished: usize,
}

/// The tile holding a point at `zoom`, in XYZ rows.
#[must_use]
pub fn tile_at(latitude_deg: f64, longitude_deg: f64, zoom: u8) -> (i32, i32) {
    let n = f64::from(1_u32 << zoom);
    let x = (longitude_deg + 180.0) / 360.0 * n;
    let lat = latitude_deg.to_radians();
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    let last = (1_i64 << zoom) - 1;
    let clamp = |value: f64| (value.floor() as i64).clamp(0, last) as i32;
    (clamp(x), clamp(y))
}

/// The square of tiles around a point at `zoom`, clipped to the world.
#[must_use]
pub fn square(latitude_deg: f64, longitude_deg: f64, zoom: u8) -> Vec<WorldTileCoords> {
    let (cx, cy) = tile_at(latitude_deg, longitude_deg, zoom);
    let last = (1_i32 << zoom) - 1;
    let mut tiles = Vec::new();
    for y in (cy - RADIUS_TILES).max(0)..=(cy + RADIUS_TILES).min(last) {
        for x in (cx - RADIUS_TILES).max(0)..=(cx + RADIUS_TILES).min(last) {
            tiles.push(WorldTileCoords::from((x, y, zoom.into())));
        }
    }
    tiles
}

/// Every tile URL the pre-warm fetches around a site.
#[must_use]
pub fn urls(config: &Config, latitude_deg: f64, longitude_deg: f64) -> Vec<String> {
    let source = |template: &str| RasterSource::from_template(template, TileAddressingScheme::XYZ);
    let base_deepest = config.base_imagery_max_zoom;
    let plan = [
        (
            source(&config.base_imagery_url),
            base_deepest.saturating_sub(BASE_LEVELS - 1)..=base_deepest,
        ),
        (
            source(&config.imagery_url),
            config.imagery_min_zoom..=config.imagery_max_zoom,
        ),
        (source(&config.dem_url), DEM_ZOOMS),
    ];
    let mut urls = Vec::new();
    for (source, zooms) in plan {
        for zoom in zooms {
            urls.extend(
                square(latitude_deg, longitude_deg, zoom)
                    .iter()
                    .filter_map(|tile| source.format(tile)),
            );
        }
    }
    urls
}

enum Outcome {
    Fetched,
    Absent,
    Failed(String),
}

async fn fetch_with_retry(client: &ReqwestHttpClient, url: &str) -> Outcome {
    let mut backoff = FIRST_BACKOFF;
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        // The error is not Send, so it is reduced before the next await.
        let result = match client.fetch(url).await {
            Ok(_) => Ok(()),
            Err(error) if error.is_not_found() => return Outcome::Absent,
            Err(error) => Err(error.describe()),
        };
        match result {
            Ok(()) => return Outcome::Fetched,
            Err(reason) => last = reason,
        }
        if attempt < ATTEMPTS {
            tokio::time::sleep(backoff).await;
            backoff = backoff.saturating_mul(2);
        }
    }
    Outcome::Failed(last)
}

/// Fetches every tile around the site into `cache_dir`, `CONCURRENCY` at a
/// time, retrying failures with backoff.
pub async fn prewarm(
    config: &Config,
    cache_dir: &str,
    latitude_deg: f64,
    longitude_deg: f64,
) -> Prewarmed {
    let client = Arc::new(ReqwestHttpClient::new(Some(cache_dir.to_owned())));
    let permits = Arc::new(Semaphore::new(CONCURRENCY));
    let started = Instant::now();
    let mut tasks = JoinSet::new();
    for url in urls(config, latitude_deg, longitude_deg) {
        let client = Arc::clone(&client);
        let permits = Arc::clone(&permits);
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await.ok();
            let outcome = fetch_with_retry(&client, &url).await;
            (url, outcome)
        });
    }
    let mut done = Prewarmed::default();
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while let Ok(Some(joined)) = tokio::time::timeout_at(deadline, tasks.join_next()).await {
        match joined {
            Ok((_, Outcome::Fetched)) => done.fetched = done.fetched.wrapping_add(1),
            Ok((_, Outcome::Absent)) => done.absent = done.absent.wrapping_add(1),
            Ok((url, Outcome::Failed(reason))) => {
                warn!(%url, %reason, "tile not cached after every attempt");
                done.failed = done.failed.wrapping_add(1);
            }
            Err(error) => {
                warn!(%error, "pre-warm task ended without a result");
                done.failed = done.failed.wrapping_add(1);
            }
        }
    }
    done.unfinished = tasks.len();
    tasks.abort_all();
    info!(
        fetched = done.fetched,
        unfinished = done.unfinished,
        absent = done.absent,
        failed = done.failed,
        seconds = started.elapsed().as_secs_f64(),
        "site tiles cached"
    );
    done
}

#[cfg(test)]
mod tests;
