//! Auditable semantic diff between two builds.
//!
//! A database update is never an opaque new binary: [`SemanticDiff::between`]
//! compares two build artifacts tile by tile — by identity and by content hash —
//! and reports which tiles were added, removed, or changed, alongside the version
//! and content-hash transition. The result is deterministically ordered and
//! serializable, so an update can be reviewed as a set of reasoned changes.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use pilotage_svs_db::{Tile, manifest_content_hash, tile_canonical_bytes, tile_leaf_hash};
use serde::Serialize;

use crate::chain::BuildArtifact;

/// How a tile changed between two builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TileChangeKind {
    /// The tile is present in the new build and absent in the old.
    Added,
    /// The tile is present in the old build and absent in the new.
    Removed,
    /// The tile is present in both but its content changed.
    Changed,
}

/// One tile's change: its identity and how it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TileDiffEntry {
    /// The tile feature-class wire code.
    pub class: u8,
    /// Tile latitude index.
    pub lat_index: i32,
    /// Tile longitude index.
    pub lon_index: i32,
    /// How the tile changed.
    pub kind: TileChangeKind,
}

/// A semantic diff between two builds of the same dataset.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticDiff {
    /// The old package version `(major, minor, patch)`.
    pub old_version: (u16, u16, u16),
    /// The new package version `(major, minor, patch)`.
    pub new_version: (u16, u16, u16),
    /// The old content hash, hex-encoded.
    pub old_content_hash: String,
    /// The new content hash, hex-encoded.
    pub new_content_hash: String,
    /// Tiles added, removed, or changed, sorted by identity.
    pub tiles: Vec<TileDiffEntry>,
}

impl SemanticDiff {
    /// Computes the diff from `old` to `new`.
    #[must_use]
    pub fn between(old: &BuildArtifact, new: &BuildArtifact) -> Self {
        let old_map = leaf_map(&old.package.tiles);
        let new_map = leaf_map(&new.package.tiles);
        let mut tiles: Vec<TileDiffEntry> = Vec::new();
        for (key, new_leaf) in &new_map {
            match old_map.get(key) {
                None => tiles.push(entry(*key, TileChangeKind::Added)),
                Some(old_leaf) if old_leaf != new_leaf => {
                    tiles.push(entry(*key, TileChangeKind::Changed));
                }
                Some(_) => {}
            }
        }
        for key in old_map.keys() {
            if !new_map.contains_key(key) {
                tiles.push(entry(*key, TileChangeKind::Removed));
            }
        }
        tiles.sort_by_key(|t| (t.class, t.lat_index, t.lon_index, kind_rank(t.kind)));
        Self {
            old_version: version_tuple(old),
            new_version: version_tuple(new),
            old_content_hash: hex32(&manifest_content_hash(&old.package.manifest)),
            new_content_hash: hex32(&manifest_content_hash(&new.package.manifest)),
            tiles,
        }
    }

    /// Whether the two builds are identical (no version, content, or tile
    /// change).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
            && self.old_version == self.new_version
            && self.old_content_hash == self.new_content_hash
    }
}

/// A map from tile identity to its leaf hash.
fn leaf_map(tiles: &[Tile]) -> BTreeMap<(u8, i32, i32), [u8; 32]> {
    tiles
        .iter()
        .map(|tile| {
            (
                (
                    tile.key.class.to_u8(),
                    tile.key.tile.lat_index,
                    tile.key.tile.lon_index,
                ),
                tile_leaf_hash(&tile_canonical_bytes(tile)),
            )
        })
        .collect()
}

/// Builds a diff entry from a tile identity and a change kind.
fn entry(key: (u8, i32, i32), kind: TileChangeKind) -> TileDiffEntry {
    TileDiffEntry {
        class: key.0,
        lat_index: key.1,
        lon_index: key.2,
        kind,
    }
}

/// A stable sort rank for a change kind.
fn kind_rank(kind: TileChangeKind) -> u8 {
    match kind {
        TileChangeKind::Added => 0,
        TileChangeKind::Removed => 1,
        TileChangeKind::Changed => 2,
    }
}

/// The package version of a build as a tuple.
fn version_tuple(artifact: &BuildArtifact) -> (u16, u16, u16) {
    let v = artifact.package.manifest.provenance.version;
    (v.major, v.minor, v.patch)
}

/// Lowercase hex of a 32-byte hash.
fn hex32(bytes: &[u8; 32]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in bytes {
        hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        hex.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    hex
}
