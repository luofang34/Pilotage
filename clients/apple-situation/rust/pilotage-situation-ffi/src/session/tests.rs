#![allow(clippy::expect_used, clippy::panic)]

use surveillance_core::{
    AddressNamespace, FieldProvenance, FieldQuality, ObservationOrigin, ObservationTime,
    ProducerInstanceId, RemovalReason, SnapshotRevision, SourceRef, TimedField, TrackDelta,
    TrackId, TrackKey, TrackRecord, TrackSnapshot, TrackSnapshotHandle, Wgs84Position,
};

use super::PresentationSession;
use crate::DisplayPointChangeKind;

#[test]
fn versioned_track_record_crosses_the_facade() {
    let session = PresentationSession::new();
    let record = track_record(9, 2, 42.36);
    let encoded = serde_json::to_string(&record).expect("track record must encode");
    let batch = session
        .accept_track_record(encoded)
        .expect("track record must be accepted");

    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.points[0].snapshot_revision, 2);
    assert_eq!(batch.point_changes.len(), 1);
    assert_eq!(batch.point_changes[0].kind, DisplayPointChangeKind::Upsert);
}

#[test]
fn stale_track_record_does_not_replace_newer_state() {
    let session = PresentationSession::new();
    for record in [track_record(9, 2, 42.36), track_record(9, 1, 41.0)] {
        let encoded = serde_json::to_string(&record).expect("track record must encode");
        session
            .accept_track_record(encoded)
            .expect("track record must be accepted");
    }
    let batch = session
        .current_display()
        .expect("display batch must be available");

    assert_eq!(batch.points[0].coordinate.latitude_deg, 42.36);
    assert!(batch.point_changes.is_empty());
}

#[test]
fn merged_record_keeps_the_transfer_target() {
    let session = PresentationSession::new();
    for record in [track_record(9, 1, 42.36), track_record(10, 2, 42.37)] {
        let encoded = serde_json::to_string(&record).expect("track record must encode");
        session
            .accept_track_record(encoded)
            .expect("track record must be accepted");
    }
    let removal = TrackRecord::new(TrackDelta::Removed {
        producer_instance_id: ProducerInstanceId::new(8),
        snapshot_revision: SnapshotRevision::new(3),
        id: TrackId::new(9),
        key: track_key(9),
        reason: RemovalReason::Merged {
            into: TrackId::new(10),
        },
    });
    let encoded = serde_json::to_string(&removal).expect("removal record must encode");
    let batch = session
        .accept_track_record(encoded)
        .expect("removal record must be accepted");

    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.point_changes[0].kind, DisplayPointChangeKind::Remove);
    assert_eq!(
        batch.point_changes[0].transfer_to.as_deref(),
        Some("traffic-8-10")
    );
}

fn track_record(id: u64, revision: u64, latitude_deg: f64) -> TrackRecord {
    let mut track = TrackSnapshot::new(TrackId::new(id), track_key(id), 10);
    track.position = Some(TimedField::new(
        Wgs84Position {
            latitude_deg,
            longitude_deg: -71.0,
        },
        ObservationTime::local(10),
        FieldQuality::default(),
        FieldProvenance::new(
            ObservationOrigin::Replay,
            AddressNamespace::Icao,
            SourceRef::default(),
        ),
    ));
    let handle = TrackSnapshotHandle::new(
        ProducerInstanceId::new(8),
        SnapshotRevision::new(revision),
        track,
    );
    TrackRecord::new(TrackDelta::Updated(handle))
}

fn track_key(id: u64) -> TrackKey {
    TrackKey::new(AddressNamespace::Icao, 0xA0_B1_00 + id as u32)
}
