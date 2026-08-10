//! Conversion from a typed Navdata snapshot to baseline features.

use std::collections::BTreeMap;

use aerocontext_core::navdata::{
    Airspace, AirspaceKind, AltitudeDatum, AltitudeLimit, ControlledClass, NavPoint, NavPointKind,
    RestrictiveKind,
};
use aerocontext_core::{Area, GeoPoint, NavDataSnapshot};
use pilotage_airspace_view::SubjectFamilyV1;

use crate::NavdataTileError;
use crate::feature::{
    BaselineFeature, BaselineGeometry, LayerKind, airway_properties, feature_id,
    standard_properties,
};
use crate::mercator::{WorldPoint, project};
use crate::model::{BaselineFeatureCounts, OmittedFeatureCounts};

pub(crate) struct SourceFeatures {
    pub(crate) features: Vec<BaselineFeature>,
    pub(crate) counts: BaselineFeatureCounts,
    pub(crate) omitted: OmittedFeatureCounts,
}

pub(crate) fn extract_features(
    snapshot: &NavDataSnapshot,
    cycle: &str,
) -> Result<SourceFeatures, NavdataTileError> {
    let point_index = PointIndex::new(&snapshot.points);
    let mut result = SourceFeatures {
        features: Vec::new(),
        counts: BaselineFeatureCounts::default(),
        omitted: OmittedFeatureCounts {
            runways_without_geometry: snapshot.runways.len() as u64,
            ..OmittedFeatureCounts::default()
        },
    };
    add_points(snapshot, cycle, &mut result)?;
    add_airways(snapshot, cycle, &point_index, &mut result)?;
    add_airspaces(snapshot, cycle, &point_index, &mut result)?;
    result.features.sort_by_key(|feature| feature.feature_id);
    Ok(result)
}

fn add_points(
    snapshot: &NavDataSnapshot,
    cycle: &str,
    result: &mut SourceFeatures,
) -> Result<(), NavdataTileError> {
    for point in &snapshot.points {
        let Some((layer, family)) = point_family(point) else {
            result.omitted.other_points = result.omitted.other_points.wrapping_add(1);
            continue;
        };
        let geometry = BaselineGeometry::Point(project(&point.ident, point.position)?);
        let mut properties = standard_properties(
            cycle,
            family,
            &point.ident,
            None,
            point.region.as_deref(),
            point.name.as_deref(),
        );
        properties.insert("kind".to_owned(), layer.name().to_owned());
        let stable_id = property(&properties, "subject_id");
        let discriminator = coordinate_key(point.position);
        result.features.push(BaselineFeature {
            layer,
            feature_id: feature_id(stable_id, &discriminator),
            properties,
            geometry,
        });
        increment_point_count(layer, &mut result.counts);
    }
    Ok(())
}

fn add_airways(
    snapshot: &NavDataSnapshot,
    cycle: &str,
    point_index: &PointIndex,
    result: &mut SourceFeatures,
) -> Result<(), NavdataTileError> {
    for airway in &snapshot.airways {
        let paths = airway_paths(airway, point_index)?;
        if paths.is_empty() {
            result.omitted.unresolved_airways = result.omitted.unresolved_airways.wrapping_add(1);
            continue;
        }
        push_airway(cycle, airway, paths, result);
    }
    Ok(())
}

fn airway_paths(
    airway: &aerocontext_core::navdata::Airway,
    point_index: &PointIndex,
) -> Result<Vec<Vec<WorldPoint>>, NavdataTileError> {
    let mut paths = Vec::new();
    let mut current = Vec::new();
    for pair in airway.points.windows(2) {
        if pair[0].gap_to_next {
            finish_path(&mut paths, &mut current);
            continue;
        }
        let Some((start, end)) = resolve_segment(airway, pair, point_index)? else {
            finish_path(&mut paths, &mut current);
            continue;
        };
        if current.last() == Some(&start) {
            current.push(end);
        } else {
            finish_path(&mut paths, &mut current);
            current.extend([start, end]);
        }
    }
    finish_path(&mut paths, &mut current);
    Ok(paths)
}

fn resolve_segment(
    airway: &aerocontext_core::navdata::Airway,
    pair: &[aerocontext_core::navdata::AirwayPoint],
    point_index: &PointIndex,
) -> Result<Option<(WorldPoint, WorldPoint)>, NavdataTileError> {
    let (Some(start), Some(end)) = (
        point_index.find(&pair[0].ident, pair[0].icao_region.as_deref()),
        point_index.find(&pair[1].ident, pair[1].icao_region.as_deref()),
    ) else {
        return Ok(None);
    };
    let start = project(&airway.ident, start)?;
    let end = project(&airway.ident, end)?;
    Ok((start != end).then_some((start, end)))
}

fn finish_path(paths: &mut Vec<Vec<WorldPoint>>, current: &mut Vec<WorldPoint>) {
    if current.len() >= 2 {
        paths.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

fn push_airway(
    cycle: &str,
    airway: &aerocontext_core::navdata::Airway,
    paths: Vec<Vec<WorldPoint>>,
    result: &mut SourceFeatures,
) {
    let location = airway.location.code();
    let mut properties = airway_properties(cycle, &airway.ident, location);
    properties.insert("kind".to_owned(), "airway".to_owned());
    let stable_id = property(&properties, "subject_id");
    result.features.push(BaselineFeature {
        layer: LayerKind::Airway,
        feature_id: feature_id(stable_id, location),
        properties,
        geometry: BaselineGeometry::Lines(paths),
    });
    result.counts.airways = result.counts.airways.wrapping_add(1);
}

fn add_airspaces(
    snapshot: &NavDataSnapshot,
    cycle: &str,
    point_index: &PointIndex,
    result: &mut SourceFeatures,
) -> Result<(), NavdataTileError> {
    for (index, airspace) in snapshot.airspaces.iter().enumerate() {
        let Some(points) = airspace_polygon(airspace, point_index) else {
            result.omitted.airspaces_without_geometry =
                result.omitted.airspaces_without_geometry.wrapping_add(1);
            continue;
        };
        let projected = points
            .into_iter()
            .map(|point| project(&airspace.designator, point))
            .collect::<Result<Vec<_>, _>>()?;
        if projected.len() < 3 {
            result.omitted.airspaces_without_geometry =
                result.omitted.airspaces_without_geometry.wrapping_add(1);
            continue;
        }
        push_airspace(cycle, airspace, projected, index, result);
    }
    Ok(())
}

fn push_airspace(
    cycle: &str,
    airspace: &Airspace,
    points: Vec<WorldPoint>,
    index: usize,
    result: &mut SourceFeatures,
) {
    let mut properties = standard_properties(
        cycle,
        SubjectFamilyV1::Airspace,
        &airspace.designator,
        airspace.center_ident.as_deref(),
        None,
        airspace.name.as_deref(),
    );
    properties.insert("kind".to_owned(), airspace_kind(&airspace.kind));
    properties.insert("lower".to_owned(), altitude(&airspace.lower));
    properties.insert("upper".to_owned(), altitude(&airspace.upper));
    properties.insert("geometry_quality".to_owned(), "snapshot_bound".to_owned());
    let stable_id = property(&properties, "subject_id");
    result.features.push(BaselineFeature {
        layer: LayerKind::Airspace,
        feature_id: feature_id(stable_id, &format!("shelf-{index}")),
        properties,
        geometry: BaselineGeometry::Polygon(points),
    });
    result.counts.airspaces = result.counts.airspaces.wrapping_add(1);
}

fn point_family(point: &NavPoint) -> Option<(LayerKind, SubjectFamilyV1)> {
    match point.kind {
        NavPointKind::Airport => Some((LayerKind::Aerodrome, SubjectFamilyV1::Aerodrome)),
        NavPointKind::Navaid => Some((LayerKind::Navaid, SubjectFamilyV1::Navaid)),
        NavPointKind::Waypoint => Some((LayerKind::Fix, SubjectFamilyV1::Fix)),
        NavPointKind::Other(_) => None,
        _ => None,
    }
}

fn increment_point_count(layer: LayerKind, counts: &mut BaselineFeatureCounts) {
    match layer {
        LayerKind::Aerodrome => counts.aerodromes = counts.aerodromes.wrapping_add(1),
        LayerKind::Navaid => counts.navaids = counts.navaids.wrapping_add(1),
        LayerKind::Fix => counts.fixes = counts.fixes.wrapping_add(1),
        LayerKind::Airway | LayerKind::Airspace => {}
    }
}

fn airspace_polygon(airspace: &Airspace, index: &PointIndex) -> Option<Vec<GeoPoint>> {
    match airspace.bounds.as_ref()? {
        Area::BoundingBox {
            south_west,
            north_east,
        } => Some(rectangle(*south_west, *north_east)),
        Area::PointRadius { center, radius_nm } => circle(*center, *radius_nm),
        Area::LocationRadius { ident, radius_nm } => circle(index.find(ident, None)?, *radius_nm),
        Area::Polygon { vertices } if vertices.len() >= 3 => Some(vertices.clone()),
        Area::Polygon { .. } => None,
        _ => None,
    }
}

fn rectangle(south_west: GeoPoint, north_east: GeoPoint) -> Vec<GeoPoint> {
    vec![
        south_west,
        GeoPoint {
            lat: south_west.lat,
            lon: north_east.lon,
        },
        north_east,
        GeoPoint {
            lat: north_east.lat,
            lon: south_west.lon,
        },
    ]
}

fn circle(center: GeoPoint, radius_nm: f64) -> Option<Vec<GeoPoint>> {
    if !radius_nm.is_finite() || radius_nm <= 0.0 {
        return None;
    }
    let latitude_radius = radius_nm / 60.0;
    let longitude_scale = center.lat.to_radians().cos().abs().max(1.0e-6);
    let longitude_radius = (latitude_radius / longitude_scale).min(180.0);
    Some(
        (0..64)
            .map(|step| {
                let angle = f64::from(step) * std::f64::consts::TAU / 64.0;
                GeoPoint {
                    lat: center.lat + latitude_radius * angle.sin(),
                    lon: center.lon + longitude_radius * angle.cos(),
                }
            })
            .collect(),
    )
}

fn airspace_kind(kind: &AirspaceKind) -> String {
    match kind {
        AirspaceKind::Controlled(class) => format!("class-{}", controlled_class(class)),
        AirspaceKind::Restrictive(kind) => restrictive_kind(kind).to_owned(),
        _ => "other".to_owned(),
    }
}

fn controlled_class(class: &ControlledClass) -> &str {
    match class {
        ControlledClass::B => "b",
        ControlledClass::C => "c",
        ControlledClass::D => "d",
        ControlledClass::E => "e",
        ControlledClass::Other(_) => "other",
        _ => "other",
    }
}

fn restrictive_kind(kind: &RestrictiveKind) -> &str {
    match kind {
        RestrictiveKind::Prohibited => "prohibited",
        RestrictiveKind::Restricted => "restricted",
        RestrictiveKind::Moa => "moa",
        RestrictiveKind::Alert => "alert",
        RestrictiveKind::Warning => "warning",
        RestrictiveKind::Danger => "danger",
        RestrictiveKind::Training => "training",
        RestrictiveKind::Other(_) => "other",
        _ => "other",
    }
}

fn altitude(limit: &AltitudeLimit) -> String {
    let datum = match limit.datum {
        AltitudeDatum::Ground => "ground",
        AltitudeDatum::Msl => "msl",
        AltitudeDatum::Agl => "agl",
        AltitudeDatum::FlightLevel => "flight-level",
        AltitudeDatum::Unlimited => "unlimited",
        AltitudeDatum::Unknown => "unknown",
        _ => "unknown",
    };
    limit
        .value_ft
        .map_or_else(|| datum.to_owned(), |value| format!("{value}:{datum}"))
}

fn coordinate_key(point: GeoPoint) -> String {
    format!("{:.12}:{:.12}", point.lat, point.lon)
}

fn property<'a>(properties: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    properties.get(name).map_or("", String::as_str)
}

#[derive(Debug, Clone, Copy)]
enum PointMatch {
    One(GeoPoint),
    Ambiguous,
}

struct PointIndex {
    exact: BTreeMap<(String, String), PointMatch>,
    any_region: BTreeMap<String, PointMatch>,
}

impl PointIndex {
    fn new(points: &[NavPoint]) -> Self {
        let mut index = Self {
            exact: BTreeMap::new(),
            any_region: BTreeMap::new(),
        };
        for point in points {
            let ident = canonical(&point.ident);
            let region = canonical(point.region.as_deref().unwrap_or(""));
            insert_point(&mut index.exact, (ident.clone(), region), point.position);
            insert_point(&mut index.any_region, ident, point.position);
        }
        index
    }

    fn find(&self, ident: &str, region: Option<&str>) -> Option<GeoPoint> {
        let ident = canonical(ident);
        let found = if let Some(region) = region {
            self.exact.get(&(ident, canonical(region)))
        } else {
            self.any_region.get(&ident)
        };
        match found {
            Some(PointMatch::One(point)) => Some(*point),
            Some(PointMatch::Ambiguous) | None => None,
        }
    }
}

fn insert_point<K: Ord>(map: &mut BTreeMap<K, PointMatch>, key: K, point: GeoPoint) {
    map.entry(key)
        .and_modify(|value| *value = PointMatch::Ambiguous)
        .or_insert(PointMatch::One(point));
}

fn canonical(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}
