#![allow(clippy::expect_used, clippy::panic)]

use png::{BitDepth, ColorType, Encoder};
use rusqlite::{Connection, params};
use surveillance_core::{
    AddressNamespace, FieldProvenance, FieldQuality, ObservationOrigin, ObservationTime,
    ProducerInstanceId, RemovalReason, SnapshotRevision, SourceRef, TimedField, TrackDelta,
    TrackId, TrackKey, TrackRecord, TrackSnapshot, TrackSnapshotHandle, Wgs84Position,
};
use tempfile::NamedTempFile;

use super::PresentationSession;
use crate::{
    DisplayLayerSourceState, DisplayPointChangeKind, PresentationRadioBand, PresentationRadioState,
    PresentationReceiverObservation, PresentationSourceObservation,
};

#[test]
fn versioned_track_record_crosses_the_facade() {
    let session = PresentationSession::new();
    let record = track_record(9, 2, 42.36);
    let encoded = serde_json::to_string(&record).expect("track record must encode");
    let batch = session
        .accept_track_record(encoded, 10)
        .expect("track record must be accepted");

    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.points[0].snapshot_revision, 2);
    assert_eq!(batch.points[0].altitude_ft, Some(5_500));
    assert_eq!(batch.point_changes.len(), 1);
    assert_eq!(batch.point_changes[0].kind, DisplayPointChangeKind::Upsert);
}

#[test]
fn loaded_terrain_places_a_track_pad_above_the_surface() {
    let archive = terrain_archive(400.0);
    let session = PresentationSession::new();
    session
        .load_terrain_archive_blocking(archive.path().to_string_lossy().into_owned())
        .expect("terrain archive must load");
    let encoded =
        serde_json::to_string(&track_record(9, 2, 42.36)).expect("track record must encode");

    let batch = session
        .accept_track_record(encoded, 10)
        .expect("track record must be accepted");

    let pad = &batch.shapes[0];
    let base = pad
        .base_above_terrain_m
        .expect("traffic pad must state its floor");
    assert!((base - (5_500.0 * 0.3048 - 400.0)).abs() < 1e-6);
    assert!(!pad.uses_reported_altitude_fallback);
}

#[test]
fn stale_track_record_does_not_replace_newer_state() {
    let session = PresentationSession::new();
    for record in [track_record(9, 2, 42.36), track_record(9, 1, 41.0)] {
        let encoded = serde_json::to_string(&record).expect("track record must encode");
        session
            .accept_track_record(encoded, 10)
            .expect("track record must be accepted");
    }
    let batch = session
        .current_display(10)
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
            .accept_track_record(encoded, 10)
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
        .accept_track_record(encoded, 10)
        .expect("removal record must be accepted");

    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.point_changes[0].kind, DisplayPointChangeKind::Remove);
    assert_eq!(
        batch.point_changes[0].transfer_to.as_deref(),
        Some("traffic-8-10")
    );
}

#[test]
fn source_observation_crosses_the_facade_without_claiming_clear_weather() {
    let session = PresentationSession::new();
    let batch = session
        .observe_sources(
            PresentationSourceObservation {
                terrain_available: true,
                radio_state: PresentationRadioState::Streaming,
                radio_receivers: vec![PresentationReceiverObservation {
                    band: PresentationRadioBand::Adsb1090,
                    state: PresentationRadioState::Streaming,
                }],
            },
            20,
        )
        .expect("source facts must be accepted");

    assert_eq!(batch.layers.len(), 4);
    let traffic = layer(&batch, "traffic");
    assert_eq!(traffic.source_state, DisplayLayerSourceState::Live);
    let weather = layer(&batch, "weather-reports");
    assert_eq!(weather.source_state, DisplayLayerSourceState::Absent);
    assert!(
        weather
            .source_detail
            .contains("does not mean clear weather")
    );
}

#[test]
fn layer_toggle_retains_domain_state_across_the_facade() {
    let session = PresentationSession::new();
    session
        .set_layer_enabled("traffic".into(), false)
        .expect("traffic layer must exist");
    let encoded =
        serde_json::to_string(&track_record(9, 2, 42.36)).expect("track record must encode");
    let hidden = session
        .accept_track_record(encoded, 10)
        .expect("track record must be accepted");
    assert!(hidden.points.is_empty());
    assert_eq!(hidden.traffic_details.len(), 1);

    let visible = session
        .set_layer_enabled("traffic".into(), true)
        .expect("traffic layer must exist");
    assert_eq!(visible.points.len(), 1);
    assert!(visible.point_changes.is_empty());
}

#[test]
fn positionless_track_crosses_as_a_list_item() {
    let session = PresentationSession::new();
    let track = TrackSnapshot::new(TrackId::new(12), track_key(12), 10);
    let record = TrackRecord::new(TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(8),
        SnapshotRevision::new(1),
        track,
    )));
    let encoded = serde_json::to_string(&record).expect("track record must encode");
    let batch = session
        .accept_track_record(encoded, 2_000_010)
        .expect("positionless track must be accepted");

    assert!(batch.points.is_empty());
    assert_eq!(batch.positionless_traffic.len(), 1);
    assert_eq!(batch.traffic_details.len(), 1);
    assert_eq!(batch.traffic_details[0].fields.len(), 8);
}

fn track_record(id: u64, revision: u64, latitude_deg: f64) -> TrackRecord {
    let mut track = TrackSnapshot::new(TrackId::new(id), track_key(id), 10);
    track.position = Some(timed(Wgs84Position {
        latitude_deg,
        longitude_deg: -71.0,
    }));
    track.pressure_altitude_ft = Some(timed(5_500));
    let handle = TrackSnapshotHandle::new(
        ProducerInstanceId::new(8),
        SnapshotRevision::new(revision),
        track,
    );
    TrackRecord::new(TrackDelta::Updated(handle))
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

fn track_key(id: u64) -> TrackKey {
    TrackKey::new(AddressNamespace::Icao, 0xA0_B1_00 + id as u32)
}

fn layer<'a>(batch: &'a crate::DisplayBatch, id: &str) -> &'a crate::DisplayLayerControl {
    batch
        .layers
        .iter()
        .find(|layer| layer.id == id)
        .expect("expected layer must exist")
}

fn terrain_archive(elevation_m: f64) -> NamedTempFile {
    let file = NamedTempFile::new().expect("create terrain fixture");
    let connection = Connection::open(file.path()).expect("open terrain fixture");
    connection
        .execute_batch(
            "CREATE TABLE metadata (name TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE tiles (
                 zoom_level INTEGER NOT NULL,
                 tile_column INTEGER NOT NULL,
                 tile_row INTEGER NOT NULL,
                 tile_data BLOB NOT NULL,
                 PRIMARY KEY (zoom_level, tile_column, tile_row)
             );",
        )
        .expect("create terrain fixture schema");
    for (name, value) in [
        ("encoding", "terrarium"),
        ("format", "png"),
        ("minzoom", "0"),
        ("maxzoom", "0"),
        ("tile_size", "2"),
    ] {
        connection
            .execute(
                "INSERT INTO metadata (name, value) VALUES (?1, ?2)",
                params![name, value],
            )
            .expect("insert terrain fixture metadata");
    }
    connection
        .execute(
            "INSERT INTO tiles (zoom_level, tile_column, tile_row, tile_data)
             VALUES (0, 0, 0, ?1)",
            [terrarium_png(elevation_m)],
        )
        .expect("insert terrain fixture tile");
    drop(connection);
    file
}

fn terrarium_png(elevation_m: f64) -> Vec<u8> {
    let code = ((elevation_m + 32_768.0) * 256.0).round() as u32;
    let pixel = [
        ((code >> 16) & 0xff) as u8,
        ((code >> 8) & 0xff) as u8,
        (code & 0xff) as u8,
    ];
    let pixels = pixel.repeat(4);
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes, 2, 2);
    encoder.set_color(ColorType::Rgb);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write terrain PNG header");
    writer
        .write_image_data(&pixels)
        .expect("write terrain PNG pixels");
    writer.finish().expect("finish terrain PNG");
    bytes
}
