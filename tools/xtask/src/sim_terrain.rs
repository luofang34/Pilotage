//! The simulator's terrain for a site: a Terrarium MBTiles archive of the
//! public AWS Terrain Tiles over a few kilometres around the site, which
//! the JSBSim sidecar stands vehicles on.
//!
//! The archive is a build artifact under `target/`, never a repository
//! file and never the client's own map archive. `cargo xtask
//! build-sim-terrain` fetches the tiles once (they are cached beside the
//! archive) and packs them. Without it, the world stands on the site's
//! stated elevation.

use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::{Connection, params};

use crate::error::XtaskError;
use crate::output::print_line;

/// Where the Terrarium tiles come from: open data, fetched without any
/// user or session information.
const TILE_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium";

/// One band of a plan: zooms and a latitude/longitude box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Band {
    pub(crate) min_zoom: u8,
    pub(crate) max_zoom: u8,
    pub(crate) south_deg: f64,
    pub(crate) north_deg: f64,
    pub(crate) west_deg: f64,
    pub(crate) east_deg: f64,
}

/// The terrain a site's simulator needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TerrainPlan {
    /// The site's name; also the archive's file stem.
    pub(crate) site: &'static str,
    pub(crate) bands: [Band; 2],
}

/// N51: about 6 by 6 km around the field at zooms 11-15 (a z15 pixel is
/// about 3.6 m there), and the tiles above it at zooms 0-10 so a vehicle
/// that leaves the box still stands on coarse terrain.
pub(crate) const N51_PLAN: TerrainPlan = TerrainPlan {
    site: "n51",
    bands: [
        Band {
            min_zoom: 0,
            max_zoom: 10,
            south_deg: 40.577,
            north_deg: 40.578,
            west_deg: -74.739,
            east_deg: -74.738,
        },
        Band {
            min_zoom: 11,
            max_zoom: 15,
            south_deg: 40.55,
            north_deg: 40.61,
            west_deg: -74.77,
            east_deg: -74.70,
        },
    ],
};

/// A tile's address: zoom, column, and XYZ row (counted from the north).
pub(crate) type TileAddress = (u8, u32, u32);
/// A fetched tile: its address and its PNG bytes.
pub(crate) type FetchedTile = (TileAddress, Vec<u8>);

/// The plans by site name.
pub(crate) const PLANS: [TerrainPlan; 1] = [N51_PLAN];

/// The archive of `site` in this repository.
pub(crate) fn archive_path(repo_root: &Path, site: &str) -> PathBuf {
    repo_root
        .join("target/sim-terrain")
        .join(format!("{site}.mbtiles"))
}

/// The Web Mercator tile (column, XYZ row) at `zoom` holding a point.
pub(crate) fn tile_of(zoom: u8, latitude_deg: f64, longitude_deg: f64) -> (u32, u32) {
    let n = f64::from(1u32 << zoom);
    let x = ((longitude_deg + 180.0) / 360.0 * n).floor();
    let lat = latitude_deg.to_radians();
    let y = ((1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor();
    let last = n - 1.0;
    (x.clamp(0.0, last) as u32, y.clamp(0.0, last) as u32)
}

/// Every tile a plan holds, without repeats.
pub(crate) fn tiles(plan: &TerrainPlan) -> Vec<TileAddress> {
    let mut tiles = Vec::new();
    for band in plan.bands {
        for zoom in band.min_zoom..=band.max_zoom {
            let (x0, y0) = tile_of(zoom, band.north_deg, band.west_deg);
            let (x1, y1) = tile_of(zoom, band.south_deg, band.east_deg);
            for x in x0..=x1 {
                for y in y0..=y1 {
                    tiles.push((zoom, x, y));
                }
            }
        }
    }
    tiles.sort_unstable();
    tiles.dedup();
    tiles
}

/// Packs fetched tiles into an MBTiles archive at `archive`, replacing it
/// whole: the archive is written beside and renamed into place.
pub(crate) fn write_archive(
    archive: &Path,
    plan: &TerrainPlan,
    tiles: &[FetchedTile],
) -> Result<(), XtaskError> {
    let partial = archive.with_extension("mbtiles.partial");
    std::fs::remove_file(&partial).ok();
    let sql = |source: rusqlite::Error| XtaskError::SimTerrain {
        context: "writing the terrain archive",
        detail: source.to_string(),
    };
    let connection = Connection::open(&partial).map_err(sql)?;
    connection
        .execute_batch(
            "CREATE TABLE metadata (name TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE tiles (zoom_level INTEGER NOT NULL, tile_column INTEGER NOT NULL,
                 tile_row INTEGER NOT NULL, tile_data BLOB NOT NULL,
                 PRIMARY KEY (zoom_level, tile_column, tile_row));",
        )
        .map_err(sql)?;
    let max_zoom = plan
        .bands
        .iter()
        .map(|band| band.max_zoom)
        .max()
        .unwrap_or(0);
    for (name, value) in [
        ("name", format!("Pilotage {} simulator terrain", plan.site)),
        ("version", "1".to_owned()),
        ("encoding", "terrarium".to_owned()),
        ("format", "png".to_owned()),
        ("type", "baselayer".to_owned()),
        ("minzoom", "0".to_owned()),
        ("maxzoom", max_zoom.to_string()),
        ("tile_size", "256".to_owned()),
        (
            "attribution",
            "Elevation from AWS Terrain Tiles. Sources include SRTM, ASTER GDEM, NRCan CDEM, \
             and USGS 3DEP."
                .to_owned(),
        ),
    ] {
        connection
            .execute(
                "INSERT INTO metadata (name, value) VALUES (?1, ?2)",
                params![name, value],
            )
            .map_err(sql)?;
    }
    for ((zoom, x, y), png) in tiles {
        // MBTiles counts rows from the south; a tile service from the north.
        let tms_row = (1u32 << zoom) - 1 - y;
        connection
            .execute(
                "INSERT INTO tiles (zoom_level, tile_column, tile_row, tile_data)
                 VALUES (?1, ?2, ?3, ?4)",
                params![zoom, x, tms_row, png],
            )
            .map_err(sql)?;
    }
    drop(connection);
    std::fs::rename(&partial, archive).map_err(|source| XtaskError::Io {
        context: "moving the terrain archive into place",
        source,
    })
}

/// Fetches one tile into `cache` unless it is there already.
fn fetch_blocking(cache: &Path, (zoom, x, y): TileAddress) -> Result<Vec<u8>, XtaskError> {
    let path = cache.join(format!("{zoom}/{x}/{y}.png"));
    if !path.is_file() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| XtaskError::Io {
                context: "creating the terrain tile cache",
                source,
            })?;
        }
        let url = format!("{TILE_URL}/{zoom}/{x}/{y}.png");
        let status = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--retry",
                "3",
                "--output",
            ])
            .arg(&path)
            .arg(&url)
            .status()
            .map_err(|source| XtaskError::Spawn {
                name: "curl",
                source,
            })?;
        if !status.success() {
            std::fs::remove_file(&path).ok();
            return Err(XtaskError::SimTerrain {
                context: "fetching a terrain tile",
                detail: format!("{url}: curl {status}"),
            });
        }
    }
    std::fs::read(&path).map_err(|source| XtaskError::Io {
        context: "reading a cached terrain tile",
        source,
    })
}

/// `cargo xtask build-sim-terrain`: fetches and packs every site's
/// simulator terrain.
///
/// # Errors
///
/// Returns [`XtaskError`] when a tile cannot be fetched or the archive
/// cannot be written.
pub(crate) fn run_build_sim_terrain(repo_root: &Path) -> Result<(), XtaskError> {
    for plan in PLANS {
        let archive = archive_path(repo_root, plan.site);
        let cache = repo_root.join("target/sim-terrain/tiles");
        let wanted = tiles(&plan);
        print_line(&format!(
            "{}: {} tiles from {TILE_URL}",
            plan.site,
            wanted.len()
        ));
        let fetched = wanted
            .iter()
            .map(|&tile| Ok((tile, fetch_blocking(&cache, tile)?)))
            .collect::<Result<Vec<_>, XtaskError>>()?;
        write_archive(&archive, &plan, &fetched)?;
        print_line(&format!(
            "{} terrain is in {}",
            plan.site,
            archive.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
