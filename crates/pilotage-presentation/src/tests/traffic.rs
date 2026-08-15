use surveillance_core::{
    AddressNamespace, AirGroundState, Band, Callsign, DeliveryPath, EmergencyState,
    FieldProvenance, FieldQuality, ObservationOrigin, ObservationTime, PositionStatus,
    FreshnessPolicy, ProducerInstanceId, ProjectionBounds, RemovalReason, SnapshotRevision, SourceRef, Squawk, TimedField, TrackDelta,
    TrackId, TrackKey, TrackPhase, TrackSnapshot, TrackSnapshotHandle, VelocityObservation,
    Wgs84Position,
};

use crate::{PointChange, PresentationAdapter, TRAFFIC_LAYER_ID};

#[test]
fn active_track_becomes_a_policy_styled_upsert() {
    let mut adapter = PresentationAdapter::new();
    let delta = updated_delta(42, 3, false, EmergencyState::None);
    let change = adapter
        .apply_traffic_delta(&delta)
        .expect("track conversion must produce a change");
    let PointChange::Upsert { point } = change else {
        panic!("active track must produce an upsert");
    };

    assert_eq!(point.id, "traffic-7-42");
    assert_eq!(point.style_id, "traffic-active");
    assert_eq!(point.label.as_deref(), Some("N42TEST\n5500 ft"));
    assert_eq!(point.rotation_deg, 92.0);
    assert_eq!(point.snapshot_revision, 3);
}

#[test]
fn pressure_altitude_is_the_traffic_comparison_value() {
    let point = point_with_altitudes(Some(5_500), Some(5_650));

    assert_eq!(point.altitude_ft, Some(5_500));
}

#[test]
fn geometric_altitude_is_the_display_fallback() {
    let point = point_with_altitudes(None, Some(5_650));

    assert_eq!(point.altitude_ft, Some(5_650));
}

#[test]
fn absent_track_altitude_stays_absent() {
    let point = point_with_altitudes(None, None);

    assert_eq!(point.altitude_ft, None);
}

#[test]
fn ownship_shadow_is_not_a_traffic_target() {
    let mut adapter = PresentationAdapter::new();
    let delta = updated_delta(42, 3, true, EmergencyState::None);

    assert!(adapter.apply_traffic_delta(&delta).is_none());
    assert!(adapter.adapt().points.is_empty());
}

#[test]
fn coasting_keeps_the_stale_vocabulary() {
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&updated_delta(42, 3, false, EmergencyState::None));
    let change = adapter
        .apply_traffic_delta(&coasting_delta(42, 4, EmergencyState::None))
        .expect("coasting must produce a change");

    assert!(matches!(
        change,
        PointChange::Stale { ref id, ref style_id, snapshot_revision: 4, .. }
            if id == "traffic-7-42" && style_id == "traffic-coasting"
    ));
    let batch = adapter.adapt();
    assert_eq!(batch.points[0].style_id, "traffic-coasting");
}

#[test]
fn emergency_style_has_priority_over_stale_style() {
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&updated_delta(42, 3, false, EmergencyState::General));
    let change = adapter
        .apply_traffic_delta(&coasting_delta(42, 4, EmergencyState::General))
        .expect("coasting must produce a change");

    assert!(matches!(
        change,
        PointChange::Stale { ref style_id, .. } if style_id == "traffic-emergency"
    ));
}

#[test]
fn merged_removal_names_the_surviving_feature() {
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&updated_delta(42, 2, false, EmergencyState::None));
    adapter.apply_traffic_delta(&updated_delta(43, 3, false, EmergencyState::None));
    let removed = TrackDelta::Removed {
        producer_instance_id: ProducerInstanceId::new(7),
        snapshot_revision: SnapshotRevision::new(4),
        id: TrackId::new(42),
        key: track_key(42),
        reason: RemovalReason::Merged {
            into: TrackId::new(43),
        },
    };
    let change = adapter
        .apply_traffic_delta(&removed)
        .expect("merged removal must produce a change");

    assert!(matches!(
        change,
        PointChange::Remove { ref id, transfer_to: Some(ref target), .. }
            if id == "traffic-7-42" && target == "traffic-7-43"
    ));
    let batch = adapter.adapt();
    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.points[0].id, "traffic-7-43");
}

#[test]
fn disabled_traffic_retains_the_newest_state_without_replay() {
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&updated_delta(42, 3, false, EmergencyState::None));
    assert!(adapter.set_layer_enabled(TRAFFIC_LAYER_ID, false));

    let hidden_change = adapter.apply_traffic_delta(&updated_delta_with_latitude(42, 4, 43.0));
    let hidden = adapter.adapt();
    assert!(hidden_change.is_none());
    assert!(hidden.points.is_empty());
    assert_eq!(hidden.traffic_details.len(), 1);

    assert!(adapter.set_layer_enabled(TRAFFIC_LAYER_ID, true));
    let visible = adapter.adapt();
    assert_eq!(visible.points.len(), 1);
    assert_eq!(visible.points[0].coordinate.latitude_deg, 43.0);
    assert!(visible.point_changes.is_empty());
}

#[test]
fn positionless_track_has_a_list_item_and_complete_absence_reasons() {
    let mut track = TrackSnapshot::new(TrackId::new(51), track_key(51), 10);
    track.position_status = PositionStatus::Current;
    track.callsign = Some(radio_timed(Callsign::new("MODESONLY")));
    track.pressure_altitude_ft = Some(radio_timed(7_000));
    let delta = TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(1),
        track,
    ));
    let mut adapter = PresentationAdapter::new();
    adapter.advance_time(2_000_010);

    assert!(adapter.apply_traffic_delta(&delta).is_none());
    let batch = adapter.adapt();
    assert!(batch.points.is_empty());
    assert_eq!(batch.positionless_traffic.len(), 1);
    assert_eq!(batch.positionless_traffic[0].title, "MODESONLY");
    let detail = &batch.traffic_details[0];
    let position = detail
        .fields
        .iter()
        .find(|field| field.id == "position")
        .expect("position field must exist");
    assert!(position.value.is_none());
    assert_eq!(
        position.absence_reason.as_deref(),
        Some("The track has no position observation.")
    );
}

#[test]
fn traffic_detail_keeps_field_age_band_and_link() {
    let mut track = complete_track(61);
    track
        .identities
        .associate(TrackKey::new(AddressNamespace::SelfAssigned, 0x00_C0DE));
    let delta = TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(2),
        track,
    ));
    let mut adapter = PresentationAdapter::new();
    adapter.advance_time(2_000_010);
    adapter.apply_traffic_delta(&delta);

    let batch = adapter.adapt();
    let detail = &batch.traffic_details[0];
    assert_eq!(detail.primary_identity, "ICAO A0B13D");
    assert_eq!(detail.other_identities, ["Self-assigned 00C0DE"]);
    assert_eq!(detail.lifecycle, "Active");
    assert_eq!(detail.newest_observation_age, "2.0 s old");
    assert_eq!(detail.fields.len(), 8);
    for field in &detail.fields {
        assert!(field.value.is_some(), "{} must have a value", field.id);
        assert_eq!(field.age.as_deref(), Some("2.0 s old"));
        let source = field.source.as_deref().expect("field source must exist");
        assert!(source.contains("1090 MHz"));
        assert!(source.contains("Local receiver"));
        assert!(field.absence_reason.is_none());
    }
}

fn updated_delta(
    id: u64,
    revision: u64,
    ownship_shadow: bool,
    emergency: EmergencyState,
) -> TrackDelta {
    TrackDelta::Updated(track_handle(
        id,
        revision,
        ownship_shadow,
        TrackPhase::Active,
        emergency,
    ))
}

fn coasting_delta(id: u64, revision: u64, emergency: EmergencyState) -> TrackDelta {
    TrackDelta::Coasting(track_handle(
        id,
        revision,
        false,
        TrackPhase::Coasting,
        emergency,
    ))
}

fn updated_delta_with_latitude(id: u64, revision: u64, latitude_deg: f64) -> TrackDelta {
    let mut track = complete_track(id);
    track.position = Some(radio_timed(Wgs84Position {
        latitude_deg,
        longitude_deg: -71.0096,
    }));
    TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(revision),
        track,
    ))
}

fn point_with_altitudes(
    pressure_altitude_ft: Option<i32>,
    geometric_altitude_ft: Option<i32>,
) -> crate::PointFeature {
    let mut track = complete_track(42);
    track.pressure_altitude_ft = pressure_altitude_ft.map(radio_timed);
    track.geometric_altitude_ft = geometric_altitude_ft.map(radio_timed);
    let delta = TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(3),
        track,
    ));
    let mut adapter = PresentationAdapter::new();
    let change = adapter
        .apply_traffic_delta(&delta)
        .expect("positioned track must produce a point");
    let PointChange::Upsert { point } = change else {
        panic!("positioned track must produce an upsert");
    };
    point
}

fn track_handle(
    id: u64,
    revision: u64,
    ownship_shadow: bool,
    phase: TrackPhase,
    emergency: EmergencyState,
) -> TrackSnapshotHandle {
    let mut track = TrackSnapshot::new(TrackId::new(id), track_key(id), 10);
    // Whether the position is usable is now a separate answer from whether the track is
    // coasting: a coasting track whose position is still current is drawn where it is.
    // These fixtures are about the pair going stale together.
    track.position_status = if phase == TrackPhase::Coasting {
        PositionStatus::Stale
    } else {
        PositionStatus::Current
    };
    track.phase = phase;
    track.ownship_shadow = ownship_shadow;
    track.position = Some(timed(Wgs84Position {
        latitude_deg: 42.3656,
        longitude_deg: -71.0096,
    }));
    track.callsign = Some(timed(Callsign::new("N42TEST")));
    track.pressure_altitude_ft = Some(timed(5_500));
    let velocity: VelocityObservation = serde_json::from_str(r#"{"track_angle_deg_true":92.0}"#)
        .expect("velocity fixture must decode");
    track.velocity = Some(timed(velocity));
    track.emergency = Some(timed(emergency));
    TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(revision),
        track,
    )
}

pub(super) fn complete_track(id: u64) -> TrackSnapshot {
    let mut track = TrackSnapshot::new(TrackId::new(id), track_key(id), 10);
    track.position_status = PositionStatus::Current;
    // A snapshot defaults to reporting no usable position, and a feature is only mapped
    // from one that does. A fixture that leaves this alone tests nothing being drawn.
    track.position_status = PositionStatus::Current;
    track.position = Some(radio_timed(Wgs84Position {
        latitude_deg: 42.3656,
        longitude_deg: -71.0096,
    }));
    track.pressure_altitude_ft = Some(radio_timed(5_500));
    track.geometric_altitude_ft = Some(radio_timed(5_650));
    track.velocity = Some(radio_timed(
        serde_json::from_str(r#"{"ground_speed_kt":120.0,"track_angle_deg_true":92.0}"#)
            .expect("velocity fixture must decode"),
    ));
    track.callsign = Some(radio_timed(Callsign::new("N61TEST")));
    track.squawk = Some(radio_timed(
        Squawk::try_from(1_200).expect("squawk fixture must be valid"),
    ));
    track.air_ground = Some(radio_timed(AirGroundState::Subsonic));
    track.emergency = Some(radio_timed(EmergencyState::None));
    track
}

fn track_key(id: u64) -> TrackKey {
    TrackKey::new(AddressNamespace::Icao, 0xA0_B1_00 + id as u32)
}

fn timed<T>(value: T) -> TimedField<T> {
    TimedField::new(
        value,
        ObservationTime::local(10),
        FieldQuality::default(),
        FieldProvenance::new(
            ObservationOrigin::Replay,
            AddressNamespace::Icao,
            SourceRef::default(),
        ),
    )
}

fn radio_timed<T>(value: T) -> TimedField<T> {
    TimedField::new(
        value,
        ObservationTime::local(10),
        FieldQuality::default(),
        FieldProvenance::new(
            ObservationOrigin::AdsbDirect {
                band: Band::Adsb1090,
            },
            AddressNamespace::Icao,
            SourceRef::new(4, 2, DeliveryPath::LocalReceiver),
        ),
    )
}

#[test]
fn a_track_with_an_altitude_draws_a_pad_at_that_height() {
    // A symbol is draped on the terrain surface whatever altitude it carries, so the pad
    // is the only part of the display that answers "above me or below me". A regression
    // here shows as traffic that silently returns to one flat plane.
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&delta_with_altitude(42, 3, Some(5_500)));

    let batch = adapter.adapt();

    assert_eq!(batch.shapes.len(), 1, "one track must draw one pad");
    let pad = &batch.shapes[0];
    assert_eq!(pad.layer_id, TRAFFIC_LAYER_ID);
    assert_eq!(pad.style_id, "traffic-altitude");
    let base = pad.base_above_terrain_m.expect("a pad states its floor");
    let top = pad.top_above_terrain_m.expect("a pad states its ceiling");
    assert!((base - 5_500.0 * 0.3048).abs() < 1e-6);
    assert!(top > base, "a pad must have thickness to be visible");
    assert!(pad.uses_reported_altitude_fallback);
    assert_eq!(pad.label.as_deref(), Some("REPORTED ALTITUDE"));
    assert_eq!(pad.rings.len(), 1);
    assert_eq!(pad.rings[0].coordinates.len(), 5);
    assert!(
        batch
            .shape_styles
            .iter()
            .any(|style| style.id == pad.style_id && style.extruded),
        "a pad must name an extruded style"
    );
}

#[test]
fn terrain_elevation_places_a_traffic_pad_above_the_surface() {
    let mut adapter = PresentationAdapter::new();
    adapter
        .apply_traffic_delta_with_terrain_blocking(&delta_with_altitude(42, 3, Some(5_500)), |_| {
            Ok::<_, std::convert::Infallible>(Some(400.0))
        })
        .expect("terrain reader is infallible");

    let batch = adapter.adapt();
    let pad = &batch.shapes[0];
    let base = pad.base_above_terrain_m.expect("a pad states its floor");

    assert!((base - (5_500.0 * 0.3048 - 400.0)).abs() < 1e-6);
    assert!(!pad.uses_reported_altitude_fallback);
    assert_eq!(pad.label, None);
}

#[test]
fn a_negative_terrain_height_keeps_the_traffic_pad() {
    let mut adapter = PresentationAdapter::new();
    adapter
        .apply_traffic_delta_with_terrain_blocking(&delta_with_altitude(42, 3, Some(1_000)), |_| {
            Ok::<_, std::convert::Infallible>(Some(400.0))
        })
        .expect("terrain reader is infallible");

    let batch = adapter.adapt();
    let base = batch.shapes[0]
        .base_above_terrain_m
        .expect("a pad states its floor");

    assert!(base < 0.0);
}

#[test]
fn a_track_without_an_altitude_stays_a_point() {
    // An invented height would claim knowledge the track never reported.
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&delta_with_altitude(42, 3, None));

    let batch = adapter.adapt();

    assert_eq!(batch.points.len(), 1);
    assert!(batch.shapes.is_empty());
}

#[test]
fn a_track_without_an_altitude_does_not_read_terrain() {
    let mut adapter = PresentationAdapter::new();
    adapter
        .apply_traffic_delta_with_terrain_blocking(&delta_with_altitude(42, 3, None), |_| {
            Err::<Option<f64>, _>("terrain must not be read")
        })
        .expect("a point without a vertical shape does not need terrain");

    assert_eq!(adapter.adapt().points.len(), 1);
}

#[test]
fn a_track_that_loses_its_altitude_loses_its_pad() {
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&delta_with_altitude(42, 3, Some(5_500)));
    assert_eq!(adapter.adapt().shapes.len(), 1);

    adapter.apply_traffic_delta(&delta_with_altitude(42, 4, None));

    assert!(adapter.adapt().shapes.is_empty());
}

fn delta_with_altitude(id: u64, revision: u64, pressure_altitude_ft: Option<i32>) -> TrackDelta {
    let mut track = complete_track(id);
    track.pressure_altitude_ft = pressure_altitude_ft.map(radio_timed);
    track.geometric_altitude_ft = None;
    TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(revision),
        track,
    ))
}

#[test]
fn a_pad_carries_the_identity_of_the_mark_it_belongs_to() {
    // The client resolves a press on a pad back to the aircraft by removing this suffix.
    // Changed here alone, a press on the only part of the target a reader sees would
    // stop finding anything, and nothing would fail.
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&delta_with_altitude(42, 3, Some(3500)));

    let batch = adapter.adapt();

    let point = batch
        .points
        .first()
        .expect("an aircraft with a height draws a mark");
    let pad = batch
        .shapes
        .iter()
        .find(|shape| shape.id.ends_with("-pad"))
        .expect("an aircraft with a height draws a pad");
    assert_eq!(pad.id, format!("{}-pad", point.id));
}

#[test]
fn a_track_is_drawn_where_it_is_now_rather_than_where_it_last_reported() {
    // The map redraws far more often than reports arrive. Drawing the reported position
    // alone is what makes a target step from one report to the next.
    let mut adapter = PresentationAdapter::new();
    let mut track = complete_track(42);
    // Taken from the producer's own defaults rather than written here, so the test
    // moves with the bounds the engine actually ships.
    track.projection_bounds = Some(ProjectionBounds::from_freshness(&FreshnessPolicy::default()));
    let velocity: VelocityObservation =
        serde_json::from_str(r#"{"track_angle_deg_true":90.0,"ground_speed_kt":300.0}"#)
            .expect("velocity fixture must decode");
    track.velocity = Some(radio_timed(velocity));
    adapter.apply_traffic_delta(&TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(3),
        track,
    )));

    let reported = adapter.adapt();
    let at_report = reported.points.first().expect("a track draws a point").clone();
    assert!(!at_report.position_is_extrapolated, "a fresh report is not a guess");

    // Five seconds later, with no new report.
    adapter.advance_time(5_000_000);
    let drawn = adapter.adapt();
    let later = drawn.points.first().expect("a track still draws a point");

    assert!(later.position_is_extrapolated, "the position must be marked as advanced");
    assert!(
        later.coordinate.longitude_deg > at_report.coordinate.longitude_deg,
        "a track heading east must be drawn further east than it last reported"
    );
}
