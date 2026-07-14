//! Tiles and the candidate package that carries them.
//!
//! A tile is identified by its feature class, level, and geographic index
//! ([`TileKey`]); the key is part of the tile's canonical bytes, so a tile
//! cannot be relabelled to a different key without changing its leaf hash. A
//! [`CandidatePackage`] is a manifest plus the tile bytes already present on the
//! device — the airborne/runtime path verifies it, it never downloads it.

use pilotage_geo::GeoTile;

use crate::feature::FeatureClass;
use crate::manifest::PackageManifest;

/// The identity of a tile: its feature class, level of detail, and geographic
/// index. Ordered class-then-level-then-index, giving tiles one canonical order
/// for the tile-root hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileKey {
    /// The feature class this tile belongs to.
    pub class: FeatureClass,
    /// The level of detail.
    pub level: u8,
    /// The geographic tile index.
    pub tile: GeoTile,
}

impl TileKey {
    /// The total-order sort key: class, level, then geographic index.
    #[must_use]
    fn order(&self) -> (u8, u8, i32, i32) {
        (
            self.class.to_u8(),
            self.level,
            self.tile.lat_index,
            self.tile.lon_index,
        )
    }
}

impl PartialOrd for TileKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TileKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.order().cmp(&other.order())
    }
}

/// A tile: its key and its opaque payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// The tile's identity.
    pub key: TileKey,
    /// The tile's opaque payload.
    pub bytes: Vec<u8>,
}

/// A manifest together with the tile bytes already present on the device. This
/// is the input to verification; the runtime path consumes it and never fetches
/// or mutates it online.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidatePackage {
    /// The package manifest.
    pub manifest: PackageManifest,
    /// The tiles the package supplies.
    pub tiles: Vec<Tile>,
}
