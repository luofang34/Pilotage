//! Deterministic canonical encoders for a tile's opaque payload bytes.
//!
//! The SVS-02 tile treats its payload as opaque bytes and binds them into its
//! leaf hash; this module fixes what those bytes are. Each encoder is
//! little-endian, length-prefixed, domain-tagged, and sorts its elements by a
//! total key before writing, so a tile's bytes are a pure function of its
//! contents and never depend on element insertion order. Floating-point values
//! are written as IEEE-754 bit patterns, matching the SVS-02 canonical style.

#[cfg(test)]
mod tests;

use crate::element::{OutputAerodrome, OutputObstacle, OutputPost, OutputRunway};

/// Domain magic for a terrain tile payload.
const TERRAIN_MAGIC: &[u8; 8] = b"SVSBTERR";
/// Domain magic for an obstacle tile payload.
const OBSTACLE_MAGIC: &[u8; 8] = b"SVSBOBST";
/// Domain magic for an aerodrome tile payload.
const AERODROME_MAGIC: &[u8; 8] = b"SVSBAERO";
/// Domain magic for a runway tile payload.
const RUNWAY_MAGIC: &[u8; 8] = b"SVSBRUNW";

/// Appends an `f64` as its little-endian IEEE-754 bit pattern.
fn push_f64(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_bits().to_le_bytes());
}

/// Encodes a terrain tile's posts, sorted by grid index `(i, j)`.
#[must_use]
pub fn encode_terrain(posts: &[OutputPost]) -> Vec<u8> {
    let mut sorted: Vec<&OutputPost> = posts.iter().collect();
    sorted.sort_by_key(|p| (p.i, p.j));
    let mut out = Vec::new();
    out.extend_from_slice(TERRAIN_MAGIC);
    out.extend_from_slice(&(sorted.len() as u64).to_le_bytes());
    for post in sorted {
        out.extend_from_slice(&post.i.to_le_bytes());
        out.extend_from_slice(&post.j.to_le_bytes());
        push_f64(&mut out, post.elevation_m);
    }
    out
}

/// Encodes an obstacle tile's obstacles, sorted by latitude, longitude, kind.
#[must_use]
pub fn encode_obstacles(obstacles: &[OutputObstacle]) -> Vec<u8> {
    let mut sorted: Vec<&OutputObstacle> = obstacles.iter().collect();
    sorted.sort_by(|a, b| {
        a.lat_deg
            .total_cmp(&b.lat_deg)
            .then(a.lon_deg.total_cmp(&b.lon_deg))
            .then(a.kind.to_u8().cmp(&b.kind.to_u8()))
    });
    let mut out = Vec::new();
    out.extend_from_slice(OBSTACLE_MAGIC);
    out.extend_from_slice(&(sorted.len() as u64).to_le_bytes());
    for obstacle in sorted {
        push_f64(&mut out, obstacle.lat_deg);
        push_f64(&mut out, obstacle.lon_deg);
        push_f64(&mut out, obstacle.height_m);
        out.push(obstacle.kind.to_u8());
    }
    out
}

/// Encodes an aerodrome tile's reference points, sorted by identifier.
#[must_use]
pub fn encode_aerodromes(aerodromes: &[OutputAerodrome]) -> Vec<u8> {
    let mut sorted: Vec<&OutputAerodrome> = aerodromes.iter().collect();
    sorted.sort_by_key(|a| a.ident);
    let mut out = Vec::new();
    out.extend_from_slice(AERODROME_MAGIC);
    out.extend_from_slice(&(sorted.len() as u64).to_le_bytes());
    for aerodrome in sorted {
        out.extend_from_slice(&aerodrome.ident.to_le_bytes());
        push_f64(&mut out, aerodrome.lat_deg);
        push_f64(&mut out, aerodrome.lon_deg);
        push_f64(&mut out, aerodrome.elevation_m);
    }
    out
}

/// Encodes a runway tile's runways, sorted by designator then endpoints.
#[must_use]
pub fn encode_runways(runways: &[OutputRunway]) -> Vec<u8> {
    let mut sorted: Vec<&OutputRunway> = runways.iter().collect();
    sorted.sort_by(|a, b| {
        a.designator
            .cmp(&b.designator)
            .then(a.end_a_lat_deg.total_cmp(&b.end_a_lat_deg))
            .then(a.end_a_lon_deg.total_cmp(&b.end_a_lon_deg))
    });
    let mut out = Vec::new();
    out.extend_from_slice(RUNWAY_MAGIC);
    out.extend_from_slice(&(sorted.len() as u64).to_le_bytes());
    for runway in sorted {
        out.extend_from_slice(&runway.designator.to_le_bytes());
        push_f64(&mut out, runway.end_a_lat_deg);
        push_f64(&mut out, runway.end_a_lon_deg);
        push_f64(&mut out, runway.end_b_lat_deg);
        push_f64(&mut out, runway.end_b_lon_deg);
    }
    out
}
