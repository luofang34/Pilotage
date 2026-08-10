//! Top-level deterministic build operation.

use pilotage_airspace_view::{IdentifiedNavdataSnapshotV1, NavdataIdentityV1};

use crate::NavdataTileError;
use crate::archive::encode_archive;
use crate::config::NavdataTileConfig;
use crate::model::{NavdataTileBundle, NavdataTileReport};
use crate::source::extract_features;
use crate::tile::partition_features;

/// Builds a deterministic vector MBTiles archive from one identified snapshot.
///
/// # Errors
///
/// Returns [`NavdataTileError`] if the configuration or identity is invalid.
/// It also returns an error if a coordinate, vector tile, compression step, or
/// MBTiles operation fails.
pub fn build_mbtiles(
    snapshot: &IdentifiedNavdataSnapshotV1,
    config: NavdataTileConfig,
) -> Result<NavdataTileBundle, NavdataTileError> {
    config.validate()?;
    validate_identity(snapshot.identity())?;
    let source = extract_features(snapshot.snapshot(), &snapshot.identity().cycle)?;
    let tiled = partition_features(&source.features, config);
    let bytes = encode_archive(snapshot.identity(), config, &tiled.tiles)?;
    let report = NavdataTileReport {
        tile_count: tiled.tiles.len() as u64,
        tile_feature_count: tiled.tile_feature_count,
        features: source.counts,
        omitted: source.omitted,
        archive_bytes: bytes.len() as u64,
    };
    Ok(NavdataTileBundle { bytes, report })
}

fn validate_identity(identity: &NavdataIdentityV1) -> Result<(), NavdataTileError> {
    for (field, value) in [
        ("cycle", identity.cycle.as_str()),
        ("snapshot_id", identity.snapshot_id.as_str()),
        ("snapshot_digest", identity.snapshot_digest.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(NavdataTileError::EmptyIdentity { field });
        }
    }
    Ok(())
}
