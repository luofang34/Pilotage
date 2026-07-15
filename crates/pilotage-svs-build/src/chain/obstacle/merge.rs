//! Deterministic obstacle merging within a tile.
//!
//! Obstacles are sorted by kind then position then source, then clustered
//! greedily: each obstacle joins the first existing cluster of its kind whose
//! anchor is within the merge tolerance, or starts a new one. The tallest
//! obstacle of a cluster is kept (ties broken by position then source), and every
//! obstacle that merged into it is recorded in the change report. The output
//! traces to every source in the cluster.

use core::cmp::Ordering;

use crate::chain::obstacle::TileObstacle;
use crate::element::OutputObstacle;
use crate::provenance::{Disposition, RecordDisposition};
use crate::source::SourceRecordRef;

/// Merges a tile's obstacles, appending merge dispositions and counting merges.
pub(crate) fn merge_tile(
    mut items: Vec<TileObstacle>,
    tolerance_deg: f64,
    dispositions: &mut Vec<RecordDisposition>,
    merged: &mut u32,
) -> Vec<OutputObstacle> {
    items.sort_by(|a, b| {
        a.kind
            .to_u8()
            .cmp(&b.kind.to_u8())
            .then(a.lat_deg.total_cmp(&b.lat_deg))
            .then(a.lon_deg.total_cmp(&b.lon_deg))
            .then(a.source.cmp(&b.source))
    });
    let mut clusters: Vec<Vec<TileObstacle>> = Vec::new();
    for item in items {
        match clusters
            .iter_mut()
            .find(|c| same_cluster(&c[0], &item, tolerance_deg))
        {
            Some(cluster) => cluster.push(item),
            None => clusters.push(vec![item]),
        }
    }
    clusters
        .into_iter()
        .map(|cluster| finish_cluster(cluster, dispositions, merged))
        .collect()
}

/// Whether `item` belongs to the cluster anchored at `anchor`: same kind and
/// within the angular tolerance.
fn same_cluster(anchor: &TileObstacle, item: &TileObstacle, tolerance_deg: f64) -> bool {
    if anchor.kind != item.kind {
        return false;
    }
    let dlat = anchor.lat_deg - item.lat_deg;
    let dlon = anchor.lon_deg - item.lon_deg;
    (dlat * dlat + dlon * dlon).sqrt() <= tolerance_deg
}

/// Resolves one cluster into a single output obstacle, recording every member
/// that merged into the kept one.
fn finish_cluster(
    cluster: Vec<TileObstacle>,
    dispositions: &mut Vec<RecordDisposition>,
    merged: &mut u32,
) -> OutputObstacle {
    let winner = pick_winner(&cluster);
    let kept = &cluster[winner];
    let mut sources: Vec<SourceRecordRef> = cluster.iter().map(|o| o.source).collect();
    sources.sort();
    sources.dedup();
    for (idx, member) in cluster.iter().enumerate() {
        if idx != winner {
            dispositions.push(RecordDisposition {
                source: member.source,
                disposition: Disposition::Merged {
                    into_lat_deg: kept.lat_deg,
                    into_lon_deg: kept.lon_deg,
                },
            });
            *merged = merged.wrapping_add(1);
        }
    }
    OutputObstacle {
        lat_deg: kept.lat_deg,
        lon_deg: kept.lon_deg,
        height_m: kept.height_m,
        kind: kept.kind,
        sources,
    }
}

/// The index of the tallest obstacle, ties broken by smallest position then
/// source, so the kept obstacle is deterministic.
fn pick_winner(cluster: &[TileObstacle]) -> usize {
    let mut best = 0usize;
    for (idx, candidate) in cluster.iter().enumerate().skip(1) {
        let incumbent = &cluster[best];
        let ordering = candidate
            .height_m
            .total_cmp(&incumbent.height_m)
            .then(incumbent.lat_deg.total_cmp(&candidate.lat_deg))
            .then(incumbent.lon_deg.total_cmp(&candidate.lon_deg))
            .then(incumbent.source.cmp(&candidate.source));
        if ordering == Ordering::Greater {
            best = idx;
        }
    }
    best
}
