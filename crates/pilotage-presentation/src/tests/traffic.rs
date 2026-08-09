use surveillance_core::{
    AddressNamespace, Callsign, EmergencyState, FieldProvenance, FieldQuality, ObservationOrigin,
    ObservationTime, ProducerInstanceId, SnapshotRevision, SourceRef, TimedField, TrackId,
    TrackKey, TrackPhase, TrackSnapshot, TrackSnapshotHandle, VelocityObservation, Wgs84Position,
};

use crate::PresentationAdapter;

#[test]
fn active_track_becomes_a_policy_styled_point() {
    let handle = track_handle(false, TrackPhase::Active, EmergencyState::None);
    let batch = PresentationAdapter::new()
        .adapt(&[handle], None)
        .expect("track conversion must succeed");
    let point = batch.points.first().expect("point must exist");

    assert_eq!(point.id, "traffic-7-42");
    assert_eq!(point.style_id, "traffic-active");
    assert_eq!(point.label.as_deref(), Some("N42TEST\n5500 ft"));
    assert_eq!(point.rotation_deg, 92.0);
    assert_eq!(point.snapshot_revision, 3);
}

#[test]
fn ownship_shadow_is_not_a_traffic_target() {
    let handle = track_handle(true, TrackPhase::Active, EmergencyState::None);
    let batch = PresentationAdapter::new()
        .adapt(&[handle], None)
        .expect("track conversion must succeed");

    assert!(batch.points.is_empty());
}

#[test]
fn emergency_style_has_priority_over_coasting_style() {
    let handle = track_handle(false, TrackPhase::Coasting, EmergencyState::General);
    let batch = PresentationAdapter::new()
        .adapt(&[handle], None)
        .expect("track conversion must succeed");

    assert_eq!(batch.points[0].style_id, "traffic-emergency");
}

fn track_handle(
    ownship_shadow: bool,
    phase: TrackPhase,
    emergency: EmergencyState,
) -> TrackSnapshotHandle {
    let key = TrackKey::new(AddressNamespace::Icao, 0xA0_B1_C2);
    let mut track = TrackSnapshot::new(TrackId::new(42), key, 10);
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
    TrackSnapshotHandle::new(ProducerInstanceId::new(7), SnapshotRevision::new(3), track)
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
