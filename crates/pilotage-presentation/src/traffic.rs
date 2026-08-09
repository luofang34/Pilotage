//! Traffic display policy.

use surveillance_geojson::{AircraftFeature, FeatureDelta};

use crate::policy::{TRAFFIC_ACTIVE_STYLE, TRAFFIC_COASTING_STYLE, TRAFFIC_EMERGENCY_STYLE};
use crate::{Coordinate, PointChange, PointFeature};

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
        FeatureDelta::Stale { id } => {
            let point = current?;
            let style_id = if point.style_id == TRAFFIC_EMERGENCY_STYLE {
                TRAFFIC_EMERGENCY_STYLE
            } else {
                TRAFFIC_COASTING_STYLE
            };
            Some(PointChange::Stale {
                id: traffic_id(producer_instance_id, id.get()),
                style_id: style_id.into(),
                producer_instance_id,
                snapshot_revision,
            })
        }
        FeatureDelta::Remove { id, transfer_to } => Some(PointChange::Remove {
            id: traffic_id(producer_instance_id, id.get()),
            transfer_to: transfer_to.map(|target| traffic_id(producer_instance_id, target.get())),
            producer_instance_id,
            snapshot_revision,
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
        coordinate,
        style_id: traffic_style(feature).into(),
        label: traffic_display_label(feature),
        rotation_deg: track
            .velocity
            .as_ref()
            .and_then(|value| value.value.track_angle_deg_true)
            .unwrap_or(0.0),
        producer_instance_id: feature.producer_instance_id().get(),
        snapshot_revision: feature.snapshot_revision().get(),
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

fn traffic_id(producer_instance_id: u64, track_id: u64) -> String {
    format!("traffic-{producer_instance_id}-{track_id}")
}
