//! Mapbox Vector Tile 2.1 Protocol Buffer encoding.

use std::collections::{BTreeMap, BTreeSet};

use prost::Message;

use crate::NavdataTileError;
use crate::feature::LayerKind;
use crate::mercator::EXTENT;
use crate::tile::VectorTile;

#[derive(Debug, Clone)]
pub(crate) enum TileGeometry {
    Point((i32, i32)),
    Lines(Vec<Vec<(i32, i32)>>),
    Polygon(Vec<(i32, i32)>),
}

#[derive(Debug, Clone)]
pub(crate) struct TileFeature {
    pub(crate) id: u64,
    pub(crate) properties: BTreeMap<String, String>,
    pub(crate) geometry: TileGeometry,
}

pub(crate) fn encode_tile(tile: &VectorTile) -> Result<Vec<u8>, NavdataTileError> {
    let layers = LayerKind::all()
        .into_iter()
        .filter_map(|kind| {
            tile.layers
                .get(&kind)
                .filter(|features| !features.is_empty())
                .map(|features| encode_layer(kind, features))
        })
        .collect();
    let message = proto::Tile { layers };
    let mut bytes = Vec::new();
    message
        .encode(&mut bytes)
        .map_err(|source| NavdataTileError::VectorTileEncoding {
            zoom: tile.coord.zoom,
            x: tile.coord.x,
            y: tile.coord.y,
            source,
        })?;
    Ok(bytes)
}

fn encode_layer(kind: LayerKind, features: &[TileFeature]) -> proto::Layer {
    let keys: Vec<String> = features
        .iter()
        .flat_map(|feature| feature.properties.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let values: Vec<String> = features
        .iter()
        .flat_map(|feature| feature.properties.values().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let key_index = indexes(&keys);
    let value_index = indexes(&values);
    let features = features
        .iter()
        .filter_map(|feature| encode_feature(feature, &key_index, &value_index))
        .collect();
    proto::Layer {
        version: 2,
        name: kind.name().to_owned(),
        features,
        keys,
        values: values
            .into_iter()
            .map(|value| proto::Value {
                string_value: Some(value),
            })
            .collect(),
        extent: Some(EXTENT as u32),
    }
}

fn encode_feature(
    feature: &TileFeature,
    key_index: &BTreeMap<String, u32>,
    value_index: &BTreeMap<String, u32>,
) -> Option<proto::Feature> {
    let (geometry_type, geometry) = encode_geometry(&feature.geometry)?;
    let tags = feature
        .properties
        .iter()
        .flat_map(|(key, value)| [key_index[key], value_index[value]])
        .collect();
    Some(proto::Feature {
        id: Some(feature.id),
        tags,
        r#type: Some(geometry_type as i32),
        geometry,
    })
}

fn encode_geometry(geometry: &TileGeometry) -> Option<(proto::GeomType, Vec<u32>)> {
    match geometry {
        TileGeometry::Point(point) => Some((proto::GeomType::Point, encode_point(*point))),
        TileGeometry::Lines(lines) => {
            let encoded = encode_lines(lines);
            (!encoded.is_empty()).then_some((proto::GeomType::Linestring, encoded))
        }
        TileGeometry::Polygon(points) => {
            encode_polygon(points).map(|encoded| (proto::GeomType::Polygon, encoded))
        }
    }
}

fn encode_point(point: (i32, i32)) -> Vec<u32> {
    vec![command(1, 1), zigzag(point.0), zigzag(point.1)]
}

fn encode_lines(lines: &[Vec<(i32, i32)>]) -> Vec<u32> {
    let mut geometry = Vec::new();
    let mut cursor = (0, 0);
    for line in lines {
        let points = deduplicate(line);
        if points.len() < 2 {
            continue;
        }
        geometry.push(command(1, 1));
        push_delta(&mut geometry, &mut cursor, points[0]);
        geometry.push(command(2, (points.len() - 1) as u32));
        for point in &points[1..] {
            push_delta(&mut geometry, &mut cursor, *point);
        }
    }
    geometry
}

fn encode_polygon(points: &[(i32, i32)]) -> Option<Vec<u32>> {
    let mut points = deduplicate(points);
    if points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 || signed_area(&points) == 0 {
        return None;
    }
    if signed_area(&points) < 0 {
        points.reverse();
    }
    let mut geometry = vec![command(1, 1), zigzag(points[0].0), zigzag(points[0].1)];
    let mut cursor = points[0];
    geometry.push(command(2, (points.len() - 1) as u32));
    for point in &points[1..] {
        push_delta(&mut geometry, &mut cursor, *point);
    }
    geometry.push(command(7, 1));
    Some(geometry)
}

fn deduplicate(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut result = Vec::with_capacity(points.len());
    for point in points {
        if result.last() != Some(point) {
            result.push(*point);
        }
    }
    result
}

fn signed_area(points: &[(i32, i32)]) -> i64 {
    let mut area = 0i64;
    for index in 0..points.len() {
        let next = index.wrapping_add(1) % points.len();
        area += i64::from(points[index].0) * i64::from(points[next].1)
            - i64::from(points[next].0) * i64::from(points[index].1);
    }
    area
}

fn push_delta(geometry: &mut Vec<u32>, cursor: &mut (i32, i32), point: (i32, i32)) {
    geometry.push(zigzag(point.0 - cursor.0));
    geometry.push(zigzag(point.1 - cursor.1));
    *cursor = point;
}

fn indexes(values: &[String]) -> BTreeMap<String, u32> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| (value.clone(), index as u32))
        .collect()
}

const fn command(id: u32, count: u32) -> u32 {
    (count << 3) | id
}

const fn zigzag(value: i32) -> u32 {
    ((value << 1) ^ (value >> 31)) as u32
}

pub(crate) mod proto {
    #[derive(Clone, PartialEq, prost::Message)]
    pub(crate) struct Tile {
        #[prost(message, repeated, tag = "3")]
        pub(crate) layers: Vec<Layer>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub(crate) struct Layer {
        #[prost(uint32, required, tag = "15")]
        pub(crate) version: u32,
        #[prost(string, required, tag = "1")]
        pub(crate) name: String,
        #[prost(message, repeated, tag = "2")]
        pub(crate) features: Vec<Feature>,
        #[prost(string, repeated, tag = "3")]
        pub(crate) keys: Vec<String>,
        #[prost(message, repeated, tag = "4")]
        pub(crate) values: Vec<Value>,
        #[prost(uint32, optional, tag = "5")]
        pub(crate) extent: Option<u32>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub(crate) struct Feature {
        #[prost(uint64, optional, tag = "1")]
        pub(crate) id: Option<u64>,
        #[prost(uint32, repeated, packed = "true", tag = "2")]
        pub(crate) tags: Vec<u32>,
        #[prost(enumeration = "GeomType", optional, tag = "3")]
        pub(crate) r#type: Option<i32>,
        #[prost(uint32, repeated, packed = "true", tag = "4")]
        pub(crate) geometry: Vec<u32>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub(crate) struct Value {
        #[prost(string, optional, tag = "1")]
        pub(crate) string_value: Option<String>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
    #[repr(i32)]
    pub(crate) enum GeomType {
        Unknown = 0,
        Point = 1,
        Linestring = 2,
        Polygon = 3,
    }
}
