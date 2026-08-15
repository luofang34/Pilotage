//! Traffic display policy.

use surveillance_geojson::{AircraftFeature, FeatureDelta};

use crate::layer::TRAFFIC_LAYER_ID;
use crate::style::{
    TRAFFIC_ACTIVE_STYLE, TRAFFIC_ALTITUDE_STYLE, TRAFFIC_COASTING_STYLE, TRAFFIC_EMERGENCY_STYLE,
};
use crate::{Coordinate, CoordinateRing, PointChange, PointFeature, ShapeFeature};

/// Half-width of one traffic pad in metres.
///
/// A pad marks where a target sits in the vertical, so it has to stay legible beside the
/// symbol without covering the ground under it.
const PAD_HALF_WIDTH_M: f64 = 220.0;

/// Thickness of one traffic pad in metres.
///
/// A pad with no thickness is invisible from directly above, which is the view a pilot
/// starts from.
const PAD_THICKNESS_M: f64 = 60.0;

const FEET_TO_METRES: f64 = 0.3048;
const METRES_PER_DEGREE_LATITUDE: f64 = 111_320.0;

/// Raise one traffic point into a pad at the altitude it reports.
///
/// A symbol is draped onto the terrain surface whatever altitude it carries, so a target
/// at 8,000 ft and one at 800 ft draw in the same place. The pad is the part of the
/// display that answers "above me or below me". A track with no altitude produces no pad
/// and keeps its symbol, because an invented height would claim knowledge the track has
/// not reported.
pub(crate) fn altitude_pad(
    point: &PointFeature,
    terrain_elevation_m: Option<f64>,
) -> Option<ShapeFeature> {
    let reported_altitude_m = f64::from(point.altitude_ft?) * FEET_TO_METRES;
    let placement = crate::vertical::reported_height(reported_altitude_m, terrain_elevation_m);
    let altitude_m = placement.metres;
    let uses_reported_altitude_fallback = placement.uses_reported_altitude_fallback;
    Some(ShapeFeature {
        id: format!("{}-pad", point.id),
        layer_id: TRAFFIC_LAYER_ID.into(),
        rings: vec![pad_ring(point.coordinate)?],
        style_id: TRAFFIC_ALTITUDE_STYLE.into(),
        label: uses_reported_altitude_fallback.then(|| "REPORTED ALTITUDE".to_owned()),
        base_above_terrain_m: Some(altitude_m),
        top_above_terrain_m: Some(altitude_m + PAD_THICKNESS_M),
        uses_reported_altitude_fallback,
        producer_instance_id: point.producer_instance_id,
        snapshot_revision: point.snapshot_revision,
    })
}

fn pad_ring(centre: Coordinate) -> Option<CoordinateRing> {
    let latitude_span = PAD_HALF_WIDTH_M / METRES_PER_DEGREE_LATITUDE;
    // A degree of longitude shortens toward the pole, so a pad built from the latitude
    // span alone would stretch east to west at high latitude.
    let longitude_scale = centre.latitude_deg.to_radians().cos();
    if longitude_scale <= f64::EPSILON {
        return None;
    }
    let longitude_span = latitude_span / longitude_scale;
    let corners = [
        (latitude_span, -longitude_span),
        (latitude_span, longitude_span),
        (-latitude_span, longitude_span),
        (-latitude_span, -longitude_span),
        (latitude_span, -longitude_span),
    ];
    let coordinates: Vec<Coordinate> = corners
        .into_iter()
        .filter_map(|(latitude_offset, longitude_offset)| {
            Coordinate::checked(
                centre.latitude_deg + latitude_offset,
                centre.longitude_deg + longitude_offset,
            )
        })
        .collect();
    (coordinates.len() == corners.len()).then_some(CoordinateRing { coordinates })
}

pub(crate) fn point_change(
    change: &FeatureDelta,
    producer_instance_id: u64,
    snapshot_revision: u64,
    current: Option<&PointFeature>,
) -> Option<PointChange> {
    match change {
        FeatureDelta::Upsert(feature) if feature.ownship_shadow() => {
            current.map(|_| PointChange::Remove {
                id: traffic_id(producer_instance_id, feature.id().get()),
                transfer_to: None,
                producer_instance_id,
                snapshot_revision,
            })
        }
        FeatureDelta::Upsert(feature) => {
            point_for_feature(feature).map(|point| PointChange::Upsert { point })
        }
        FeatureDelta::Stale { id, revision } => {
            let point = current?;
            let style_id = if point.style_id == TRAFFIC_EMERGENCY_STYLE {
                TRAFFIC_EMERGENCY_STYLE
            } else {
                TRAFFIC_COASTING_STYLE
            };
            Some(PointChange::Stale {
                id: traffic_id(producer_instance_id, id.track_id().get()),
                style_id: style_id.into(),
                producer_instance_id,
                snapshot_revision: revision.get(),
            })
        }
        FeatureDelta::Remove {
            id,
            transfer_to,
            revision,
        } => Some(PointChange::Remove {
            id: traffic_id(producer_instance_id, id.track_id().get()),
            transfer_to: transfer_to
                .map(|target| traffic_id(producer_instance_id, target.track_id().get())),
            producer_instance_id,
            snapshot_revision: revision.get(),
        }),
        _ => None,
    }
}

fn point_for_feature(feature: &AircraftFeature) -> Option<PointFeature> {
    let position = feature.position();
    let coordinate =
        Coordinate::checked(position.value.latitude_deg, position.value.longitude_deg)?;
    let track = feature.snapshot();
    Some(PointFeature {
        id: traffic_id(feature.producer_instance_id().get(), feature.id().get()),
        layer_id: TRAFFIC_LAYER_ID.into(),
        coordinate,
        style_id: traffic_style(feature).into(),
        label: traffic_display_label(feature),
        altitude_ft: display_altitude_ft(feature),
        rotation_deg: track
            .velocity
            .as_ref()
            .and_then(|value| value.value.track_angle_deg_true)
            .unwrap_or(0.0),
        position_is_extrapolated: false,
        producer_instance_id: feature.producer_instance_id().get(),
        snapshot_revision: feature.snapshot_revision().get(),
    })
}

fn display_altitude_ft(feature: &AircraftFeature) -> Option<i32> {
    let track = feature.snapshot();
    // Traffic displays compare the pressure altitude that transponders report.
    track
        .pressure_altitude_ft
        .as_ref()
        .map(|field| field.value)
        .or_else(|| {
            track
                .geometric_altitude_ft
                .as_ref()
                .map(|field| field.value)
        })
}

fn traffic_style(feature: &AircraftFeature) -> &'static str {
    let emergency = feature
        .snapshot()
        .emergency
        .as_ref()
        .is_some_and(|field| field.value.is_active());
    if emergency {
        TRAFFIC_EMERGENCY_STYLE
    } else {
        TRAFFIC_ACTIVE_STYLE
    }
}

fn traffic_label(feature: &AircraftFeature) -> Option<String> {
    let track = feature.snapshot();
    track
        .callsign
        .as_ref()
        .map(|field| field.value.text.trim().to_owned())
        .filter(|label| !label.is_empty())
        .or_else(|| Some(format!("{:06X}", track.key.address)))
}

fn altitude_label(feature: &AircraftFeature) -> Option<String> {
    let track = feature.snapshot();
    track
        .pressure_altitude_ft
        .as_ref()
        .map(|field| format!("{} ft", field.value))
        .or_else(|| {
            track
                .geometric_altitude_ft
                .as_ref()
                .map(|field| format!("{} ft GNSS", field.value))
        })
}

fn traffic_display_label(feature: &AircraftFeature) -> Option<String> {
    let primary = traffic_label(feature)?;
    Some(match altitude_label(feature) {
        Some(altitude) => format!("{primary}\n{altitude}"),
        None => primary,
    })
}

pub(crate) fn traffic_id(producer_instance_id: u64, track_id: u64) -> String {
    format!("traffic-{producer_instance_id}-{track_id}")
}
