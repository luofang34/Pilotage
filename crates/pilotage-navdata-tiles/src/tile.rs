//! Spatial partitioning into clipped vector tiles.

use std::collections::BTreeMap;

use crate::config::NavdataTileConfig;
use crate::feature::{BaselineFeature, BaselineGeometry, LayerKind};
use crate::geometry::{clip_polygon, clip_segment, point_bounds};
use crate::mercator::{
    TileCoord, WorldPoint, bounds, local_point, tile_for_point, tiles_for_bounds,
};
use crate::mvt::{TileFeature, TileGeometry};

pub(crate) struct VectorTile {
    pub(crate) coord: TileCoord,
    pub(crate) layers: BTreeMap<LayerKind, Vec<TileFeature>>,
}

pub(crate) struct TiledFeatures {
    pub(crate) tiles: Vec<VectorTile>,
    pub(crate) tile_feature_count: u64,
}

type PendingTiles = BTreeMap<TileCoord, BTreeMap<LayerKind, Vec<TileFeature>>>;

pub(crate) fn partition_features(
    features: &[BaselineFeature],
    config: NavdataTileConfig,
) -> TiledFeatures {
    let mut pending = PendingTiles::new();
    for feature in features {
        let minimum = feature.layer.min_zoom(config);
        for zoom in minimum..=config.max_zoom {
            match &feature.geometry {
                BaselineGeometry::Point(point) => add_point(&mut pending, feature, *point, zoom),
                BaselineGeometry::Lines(lines) => {
                    add_lines(&mut pending, feature, lines, zoom);
                }
                BaselineGeometry::Polygon(points) => {
                    add_polygon(&mut pending, feature, points, zoom);
                }
            }
        }
    }
    let tile_feature_count = pending
        .values()
        .flat_map(BTreeMap::values)
        .map(|features| features.len() as u64)
        .sum();
    let tiles = pending
        .into_iter()
        .map(|(coord, layers)| VectorTile { coord, layers })
        .collect();
    TiledFeatures {
        tiles,
        tile_feature_count,
    }
}

fn add_point(pending: &mut PendingTiles, feature: &BaselineFeature, point: WorldPoint, zoom: u8) {
    let tile = tile_for_point(point, zoom);
    push_feature(
        pending,
        tile,
        feature,
        TileGeometry::Point(local_point(point, tile)),
    );
}

fn add_lines(
    pending: &mut PendingTiles,
    feature: &BaselineFeature,
    lines: &[Vec<WorldPoint>],
    zoom: u8,
) {
    let mut by_tile: BTreeMap<TileCoord, Vec<Vec<(i32, i32)>>> = BTreeMap::new();
    for line in lines {
        let mut line_by_tile: BTreeMap<TileCoord, Vec<Vec<(i32, i32)>>> = BTreeMap::new();
        for segment in line.windows(2) {
            let Some((min, max)) = point_bounds(segment) else {
                continue;
            };
            for tile in tiles_for_bounds(min, max, zoom) {
                let Some((start, end)) = clip_segment(segment[0], segment[1], bounds(tile)) else {
                    continue;
                };
                let local = vec![local_point(start, tile), local_point(end, tile)];
                if local[0] != local[1] {
                    append_segment(line_by_tile.entry(tile).or_default(), local);
                }
            }
        }
        for (tile, parts) in line_by_tile {
            by_tile.entry(tile).or_default().extend(parts);
        }
    }
    for (tile, parts) in by_tile {
        push_feature(pending, tile, feature, TileGeometry::Lines(parts));
    }
}

fn append_segment(parts: &mut Vec<Vec<(i32, i32)>>, segment: Vec<(i32, i32)>) {
    if let Some(part) = parts
        .last_mut()
        .filter(|part| part.last() == segment.first())
    {
        part.extend_from_slice(&segment[1..]);
    } else {
        parts.push(segment);
    }
}

fn add_polygon(
    pending: &mut PendingTiles,
    feature: &BaselineFeature,
    points: &[WorldPoint],
    zoom: u8,
) {
    let Some((min, max)) = point_bounds(points) else {
        return;
    };
    for tile in tiles_for_bounds(min, max, zoom) {
        let clipped = clip_polygon(points, bounds(tile));
        if clipped.len() < 3 {
            continue;
        }
        let local: Vec<_> = clipped
            .into_iter()
            .map(|point| local_point(point, tile))
            .collect();
        if polygon_is_drawable(&local) {
            push_feature(pending, tile, feature, TileGeometry::Polygon(local));
        }
    }
}

fn push_feature(
    pending: &mut PendingTiles,
    tile: TileCoord,
    feature: &BaselineFeature,
    geometry: TileGeometry,
) {
    pending
        .entry(tile)
        .or_default()
        .entry(feature.layer)
        .or_default()
        .push(TileFeature {
            id: feature.feature_id,
            properties: feature.properties.clone(),
            geometry,
        });
}

fn polygon_is_drawable(points: &[(i32, i32)]) -> bool {
    let mut unique = Vec::new();
    for point in points {
        if unique.last() != Some(point) {
            unique.push(*point);
        }
    }
    if unique.first() == unique.last() {
        unique.pop();
    }
    if unique.len() < 3 {
        return false;
    }
    let mut area = 0i64;
    for index in 0..unique.len() {
        let next = index.wrapping_add(1) % unique.len();
        area += i64::from(unique[index].0) * i64::from(unique[next].1)
            - i64::from(unique[next].0) * i64::from(unique[index].1);
    }
    area != 0
}
