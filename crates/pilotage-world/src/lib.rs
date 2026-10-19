//! One simulated world for every simulator and flight controller of a
//! session: the site vehicles stand on, the terrain under it, and the
//! magnetic field over it.
//!
//! A dynamics model takes its ground from [`World::elevation_blocking`],
//! a sensor model its magnetic field from [`World::magnetic_field`], and
//! a launcher its vehicle placements from [`World::placement_blocking`],
//! so every vehicle and every flight controller of one session sees the
//! same ground and the same field.
//!
//! The crate is sans-IO apart from the optional `archive` terrain source,
//! and is `no_std` without its `std` feature.

#![cfg_attr(not(any(feature = "std", test)), no_std)]

mod error;
#[cfg(all(test, feature = "archive"))]
mod fixture;
mod geomag;
mod site;
mod terrain;
mod world;

pub use error::{TerrainError, WorldError};
pub use geomag::{GeomagModel, MagneticEpoch, MagneticField, WMM2025};
pub use site::{
    DEFAULT_SITE, Ground, N51, NAMED_SITES, Placement, Site, ZURICH, metres_per_degree,
};
#[cfg(feature = "archive")]
pub use terrain::ArchiveTerrain;
pub use terrain::{TerrainSample, TerrainSource, UNIFORM_TERRAIN_ID, UniformTerrain, WORLD_GEOID};
pub use world::{ElevationOrigin, SiteElevation, World, WorldSources};
