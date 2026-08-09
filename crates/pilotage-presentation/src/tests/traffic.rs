use surveillance_core::{
    AddressNamespace, Callsign, EmergencyState, FieldProvenance, FieldQuality, ObservationOrigin,
    ObservationTime, ProducerInstanceId, RemovalReason, SnapshotRevision, SourceRef, TimedField,
    TrackDelta, TrackId, TrackKey, TrackPhase, TrackSnapshot, TrackSnapshotHandle,
    VelocityObservation, Wgs84Position,
};

use crate::{PointChange, PresentationAdapter};

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
fn ownship_shadow_is_not_a_traffic_target() {
    let mut adapter = PresentationAdapter::new();
    let delta = updated_delta(42, 3, true, EmergencyState::None);

    assert!(adapter.apply_traffic_delta(&delta).is_none());
    assert!(
        adapter
            .adapt(None)
            .expect("batch conversion must succeed")
            .points
            .is_empty()
    );
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
    let batch = adapter.adapt(None).expect("batch conversion must succeed");
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
    let batch = adapter.adapt(None).expect("batch conversion must succeed");
    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.points[0].id, "traffic-7-43");
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

fn track_handle(
    id: u64,
    revision: u64,
    ownship_shadow: bool,
    phase: TrackPhase,
    emergency: EmergencyState,
) -> TrackSnapshotHandle {
    let mut track = TrackSnapshot::new(TrackId::new(id), track_key(id), 10);
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
