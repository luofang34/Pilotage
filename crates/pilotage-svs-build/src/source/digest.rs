//! An immutable content digest over a source's declared input.
//!
//! [`source_content_digest`] hashes a source's version, declared frame, and every
//! input record it contributes, in a fixed canonical order. Changing one byte of
//! any input record — an elevation post, an obstacle height, a runway endpoint —
//! changes the digest. The build records the digest in the signed provenance, so
//! the source inputs are bound into the signature and a provenance whose digest
//! disagrees with the source is detectable.

use sha2::{Digest, Sha256};

use super::{SourceDataset, SourceId, SourceMeta, SourceRecordKey, SourceRecordRef};

/// Domain-separating magic for a source's canonical input bytes.
const SOURCE_MAGIC: &[u8; 8] = b"SVSBSRC0";

/// The SHA-256 content digest of a source's declared input.
#[must_use]
pub(crate) fn source_content_digest(dataset: &SourceDataset, meta: &SourceMeta) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(canonical_source_bytes(dataset, meta));
    hasher.finalize().into()
}

/// The canonical bytes of a source's input: its declaration then its records.
fn canonical_source_bytes(dataset: &SourceDataset, meta: &SourceMeta) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(SOURCE_MAGIC);
    append_meta(&mut out, meta);
    append_terrain(&mut out, dataset, meta.id);
    append_obstacles(&mut out, dataset, meta.id);
    append_aerodromes(&mut out, dataset, meta.id);
    out
}

/// Appends an `f64` as its little-endian IEEE-754 bit pattern.
fn push_f64(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_bits().to_le_bytes());
}

/// Appends the source declaration: version, license, datum frame, accuracy.
fn append_meta(out: &mut Vec<u8>, meta: &SourceMeta) {
    out.extend_from_slice(&meta.version.to_le_bytes());
    out.push(meta.license.to_u8());
    out.push(meta.horizontal_datum.to_u8());
    out.extend_from_slice(&meta.realization.0.to_le_bytes());
    out.push(meta.vertical_datum.to_u8());
    out.extend_from_slice(&meta.geoid.0.to_le_bytes());
    out.extend_from_slice(&meta.accuracy.horizontal_mm.to_le_bytes());
    out.extend_from_slice(&meta.accuracy.vertical_mm.to_le_bytes());
}

/// Appends this source's terrain grids and posts, in a fixed order.
fn append_terrain(out: &mut Vec<u8>, dataset: &SourceDataset, id: SourceId) {
    let mut grids: Vec<&super::TerrainGrid> =
        dataset.terrain.iter().filter(|g| g.source == id).collect();
    grids.sort_by(|a, b| {
        a.origin_lat_deg
            .total_cmp(&b.origin_lat_deg)
            .then(a.origin_lon_deg.total_cmp(&b.origin_lon_deg))
    });
    out.extend_from_slice(&(grids.len() as u64).to_le_bytes());
    for grid in grids {
        push_f64(out, grid.origin_lat_deg);
        push_f64(out, grid.origin_lon_deg);
        push_f64(out, grid.step_deg);
        out.extend_from_slice(&grid.rows.to_le_bytes());
        out.extend_from_slice(&grid.cols.to_le_bytes());
        for post in &grid.posts {
            match post {
                Some(height) => {
                    out.push(1);
                    push_f64(out, *height);
                }
                None => out.push(0),
            }
        }
    }
}

/// Appends a source record reference's class and key, so a record's identity is
/// part of the digest.
fn push_source_ref(out: &mut Vec<u8>, source_ref: &SourceRecordRef) {
    out.push(source_ref.class_code());
    match source_ref.key {
        SourceRecordKey::TerrainNode {
            grid_lat_bits,
            grid_lon_bits,
            row,
            col,
        } => {
            out.extend_from_slice(&grid_lat_bits.to_le_bytes());
            out.extend_from_slice(&grid_lon_bits.to_le_bytes());
            out.extend_from_slice(&row.to_le_bytes());
            out.extend_from_slice(&col.to_le_bytes());
        }
        SourceRecordKey::Obstacle { index } => out.extend_from_slice(&index.to_le_bytes()),
        SourceRecordKey::Aerodrome { ident } => out.extend_from_slice(&ident.to_le_bytes()),
        SourceRecordKey::Runway {
            aerodrome,
            designator,
        } => {
            out.extend_from_slice(&aerodrome.to_le_bytes());
            out.extend_from_slice(&designator.to_le_bytes());
        }
    }
}

/// Appends this source's obstacles, ordered by their reference.
fn append_obstacles(out: &mut Vec<u8>, dataset: &SourceDataset, id: SourceId) {
    let mut obstacles: Vec<&super::Obstacle> = dataset
        .obstacles
        .iter()
        .filter(|o| o.source.source == id)
        .collect();
    obstacles.sort_by_key(|o| o.source);
    out.extend_from_slice(&(obstacles.len() as u64).to_le_bytes());
    for obstacle in obstacles {
        push_source_ref(out, &obstacle.source);
        push_f64(out, obstacle.lat_deg);
        push_f64(out, obstacle.lon_deg);
        push_f64(out, obstacle.height_m);
        out.push(obstacle.kind.to_u8());
    }
}

/// Appends this source's aerodromes and their runways, ordered by identifier.
fn append_aerodromes(out: &mut Vec<u8>, dataset: &SourceDataset, id: SourceId) {
    let mut aerodromes: Vec<&super::Aerodrome> = dataset
        .aerodromes
        .iter()
        .filter(|a| a.source.source == id)
        .collect();
    aerodromes.sort_by_key(|a| a.ident);
    out.extend_from_slice(&(aerodromes.len() as u64).to_le_bytes());
    for aerodrome in aerodromes {
        out.extend_from_slice(&aerodrome.ident.to_le_bytes());
        push_f64(out, aerodrome.ref_lat_deg);
        push_f64(out, aerodrome.ref_lon_deg);
        push_f64(out, aerodrome.elevation_m);
        push_source_ref(out, &aerodrome.source);
        append_runways(out, &aerodrome.runways);
    }
}

/// Appends an aerodrome's runways, ordered by designator.
fn append_runways(out: &mut Vec<u8>, runways: &[super::Runway]) {
    let mut ordered: Vec<&super::Runway> = runways.iter().collect();
    ordered.sort_by_key(|r| r.designator);
    out.extend_from_slice(&(ordered.len() as u64).to_le_bytes());
    for runway in ordered {
        out.extend_from_slice(&runway.designator.to_le_bytes());
        push_f64(out, runway.end_a_lat_deg);
        push_f64(out, runway.end_a_lon_deg);
        push_f64(out, runway.end_b_lat_deg);
        push_f64(out, runway.end_b_lon_deg);
        push_source_ref(out, &runway.source);
    }
}
