#![allow(clippy::expect_used, clippy::panic)]

use surveillance_core::{
    AddressNamespace, FieldProvenance, FieldQuality, ObservationOrigin, ObservationTime,
    ProducerInstanceId, SnapshotRevision, SourceRef, TimedField, TrackDelta, TrackId, TrackKey,
    TrackRecord, TrackSnapshot, TrackSnapshotHandle, Wgs84Position,
};

use super::PresentationSession;

#[test]
fn versioned_track_record_crosses_the_facade() {
    let session = PresentationSession::new();
    let record = track_record(2, 42.36);
    let encoded = serde_json::to_string(&record).expect("track record must encode");
    let batch = session
        .accept_track_record(encoded)
        .expect("track record must be accepted");

    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.points[0].snapshot_revision, 2);
}

#[test]
fn stale_track_record_does_not_replace_newer_state() {
    let session = PresentationSession::new();
    for record in [track_record(2, 42.36), track_record(1, 41.0)] {
        let encoded = serde_json::to_string(&record).expect("track record must encode");
        session
            .accept_track_record(encoded)
            .expect("track record must be accepted");
    }
    let batch = session
        .current_display()
        .expect("display batch must be available");

    assert_eq!(batch.points[0].coordinate.latitude_deg, 42.36);
}

fn track_record(revision: u64, latitude_deg: f64) -> TrackRecord {
    let key = TrackKey::new(AddressNamespace::Icao, 0xA0_B1_C2);
    let mut track = TrackSnapshot::new(TrackId::new(9), key, 10);
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
