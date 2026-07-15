//! Enumerating every source record's canonical reference.
//!
//! [`source_record_refs`] lists the unambiguous [`SourceRecordRef`] of every
//! record the dataset contains: each terrain grid node, each obstacle, each
//! aerodrome, and each runway. A verifier uses this to resolve a lineage
//! reference to exactly one source record and to prove no two distinct source
//! records share a reference (a collision here is an ambiguous dataset).

use super::{SourceDataset, SourceRecordRef};

/// Every source record's reference, in dataset order.
#[must_use]
pub(crate) fn source_record_refs(dataset: &SourceDataset) -> Vec<SourceRecordRef> {
    let mut refs = Vec::new();
    for grid in &dataset.terrain {
        for row in 0..grid.rows {
            for col in 0..grid.cols {
                refs.push(SourceRecordRef::terrain(
                    grid.source,
                    grid.origin_lat_deg,
                    grid.origin_lon_deg,
                    row,
                    col,
                ));
            }
        }
    }
    for obstacle in &dataset.obstacles {
        refs.push(obstacle.source);
    }
    for aerodrome in &dataset.aerodromes {
        refs.push(aerodrome.source);
        for runway in &aerodrome.runways {
            refs.push(runway.source);
        }
    }
    refs
}
