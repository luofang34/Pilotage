//! Traffic display policy.

use surveillance_core::{TrackPhase, TrackSnapshot, TrackSnapshotHandle};

use crate::policy::{TRAFFIC_ACTIVE_STYLE, TRAFFIC_COASTING_STYLE, TRAFFIC_EMERGENCY_STYLE};
use crate::{Coordinate, PointFeature};

pub(crate) fn point_for_track(handle: &TrackSnapshotHandle) -> Option<PointFeature> {
    let track = handle.snapshot();
    if track.ownship_shadow {
        return None;
    }
    let position = track.position.as_ref()?;
    let coordinate =
        Coordinate::checked(position.value.latitude_deg, position.value.longitude_deg)?;
    let label = traffic_display_label(track);
    Some(PointFeature {
        id: format!(
            "traffic-{}-{}",
            handle.producer_instance_id().get(),
            track.id.get()
        ),
        coordinate,
        style_id: traffic_style(track).into(),
        label,
        rotation_deg: track
            .velocity
            .as_ref()
            .and_then(|value| value.value.track_angle_deg_true)
            .unwrap_or(0.0),
        producer_instance_id: handle.producer_instance_id().get(),
        snapshot_revision: handle.snapshot_revision().get(),
    })
}

fn traffic_style(track: &TrackSnapshot) -> &'static str {
    let emergency = track
        .emergency
        .as_ref()
        .is_some_and(|field| field.value.is_active());
    if emergency {
        TRAFFIC_EMERGENCY_STYLE
    } else {
        match track.phase {
            TrackPhase::Active => TRAFFIC_ACTIVE_STYLE,
            TrackPhase::Coasting => TRAFFIC_COASTING_STYLE,
            _ => TRAFFIC_COASTING_STYLE,
        }
    }
}

fn traffic_label(track: &TrackSnapshot) -> Option<String> {
    track
        .callsign
        .as_ref()
        .map(|field| field.value.text.trim().to_owned())
        .filter(|label| !label.is_empty())
        .or_else(|| Some(format!("{:06X}", track.key.address)))
}

fn altitude_label(track: &TrackSnapshot) -> Option<String> {
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

fn traffic_display_label(track: &TrackSnapshot) -> Option<String> {
    let primary = traffic_label(track)?;
    Some(match altitude_label(track) {
        Some(altitude) => format!("{primary}\n{altitude}"),
        None => primary,
    })
}
