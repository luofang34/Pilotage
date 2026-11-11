//! The archive checks. An archive is a build artifact, so each check
//! runs only when the working tree holds the archive.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::json::{field, file_digest, items, or_else, raw_text};
use super::{ARCHIVE, COASTLINE_ARCHIVE, Inputs, require};

/// The vector layers the coastline archive must hold, sorted.
const COASTLINE_LAYERS: [&str; 4] = ["lakes", "land", "ocean", "rivers"];

fn open(path: &Path) -> Option<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
}

/// The number of tiles at `zoom`, or `None` when the archive does not
/// answer.
fn tiles_at(path: &Path, zoom: u32) -> Option<u64> {
    let count: i64 = open(path)?
        .query_row(
            "SELECT COUNT(*) FROM tiles WHERE zoom_level = ?1;",
            [zoom],
            |row| row.get(0),
        )
        .ok()?;
    u64::try_from(count).ok()
}

/// The archive holds what the plan promises at its deepest zoom: a band
/// that covers the world holds at least half of that zoom's tiles. Half
/// is far below what a real build produces and far above what a regional
/// band can reach.
pub(super) fn check_coastline_coverage(inputs: &Inputs, refusals: &mut Vec<String>) {
    let path = inputs.paths.at(COASTLINE_ARCHIVE);
    if !path.is_file() {
        return;
    }
    let deepest = field(&inputs.coastline_plan, "closest_zoom")
        .as_f64()
        .filter(|zoom| zoom.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(zoom))
        .and_then(|zoom| format!("{zoom}").parse::<u32>().ok());
    let Some(count) = deepest.and_then(|zoom| tiles_at(&path, zoom)) else {
        require(
            false,
            "the coastline archive did not answer for its deepest zoom",
            refusals,
        );
        return;
    };
    let expected = deepest
        .and_then(|zoom| 4_u64.checked_pow(zoom))
        .unwrap_or(u64::MAX);
    require(
        count >= expected / 2,
        "the coastline archive does not cover the world at its deepest zoom",
        refusals,
    );
}

/// Each archive that exists matches its manifest, and the coastline
/// archive holds the four vector layers.
pub(super) fn check_digests(inputs: &Inputs, refusals: &mut Vec<String>) {
    let empty = Value::String(String::new());
    let terrain = inputs.paths.at(ARCHIVE);
    if terrain.is_file() {
        require(
            file_digest(&terrain)
                == raw_text(or_else(field(&inputs.manifest, "archive_sha256"), &empty)),
            "the terrain archive does not match its manifest",
            refusals,
        );
    }
    let coastline = inputs.paths.at(COASTLINE_ARCHIVE);
    if coastline.is_file() {
        require(
            file_digest(&coastline)
                == raw_text(or_else(
                    field(&inputs.coastline_manifest, "archive_sha256"),
                    &empty,
                )),
            "the coastline archive does not match its manifest",
            refusals,
        );
        require(
            holds_coastline_layers(&coastline),
            "the coastline archive must contain ocean, land, lakes, and rivers",
            refusals,
        );
    }
}

/// The archive's `json` metadata names exactly the four vector layers.
/// Every `json` row is read, as `sqlite3` prints them all; the last
/// answer decides.
fn holds_coastline_layers(path: &Path) -> bool {
    let Some(connection) = open(path) else {
        return false;
    };
    let Ok(mut statement) = connection.prepare("SELECT value FROM metadata WHERE name = 'json';")
    else {
        return false;
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
        return false;
    };
    let answers: Vec<bool> = rows
        .map(|row| {
            row.ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .is_some_and(|metadata| names_the_layers(&metadata))
        })
        .collect();
    answers.last().copied().unwrap_or(false)
}

fn names_the_layers(metadata: &Value) -> bool {
    let Some(layers) = items(field(metadata, "vector_layers")) else {
        return false;
    };
    let mut ids = Vec::with_capacity(layers.len());
    for layer in layers {
        let Some(id) = field(layer, "id").as_str() else {
            return false;
        };
        ids.push(id);
    }
    ids.sort_unstable();
    ids == COASTLINE_LAYERS
}
