use surveillance_core::{ProducerInstanceId, SnapshotRevision, TrackDelta, TrackSnapshotHandle};

use crate::PresentationAdapter;

use super::traffic::complete_track;

#[test]
fn the_aircraft_own_return_becomes_a_position_and_not_traffic() {
    // The receiver hears the aircraft's own transmission. Drawing it as traffic would put
    // a target on top of the aircraft; throwing it away loses the one position the client
    // can be sure belongs to this aircraft.
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&ownship_delta(42, 3));

    let batch = adapter.adapt();

    assert!(batch.points.is_empty(), "the aircraft is not traffic");
    let ownship = batch
        .ownship
        .expect("the aircraft's own return carries a position");
    assert!((ownship.coordinate.latitude_deg - 42.3656).abs() < 1e-9);
    assert!((ownship.coordinate.longitude_deg + 71.0096).abs() < 1e-9);
}

#[test]
fn an_own_return_with_no_position_reports_none() {
    // Heard is not located. A coordinate invented here would be drawn as the aircraft.
    let mut adapter = PresentationAdapter::new();
    let mut track = complete_track(42);
    track.position = None;
    track.ownship_shadow = true;
    adapter.apply_traffic_delta(&TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(3),
        track,
    )));

    assert!(adapter.adapt().ownship.is_none());
}

#[test]
fn clearing_radio_state_forgets_where_the_aircraft_was() {
    let mut adapter = PresentationAdapter::new();
    adapter.apply_traffic_delta(&ownship_delta(42, 3));
    assert!(adapter.adapt().ownship.is_some());

    adapter.clear_radio_state();

    assert!(adapter.adapt().ownship.is_none());
}

fn ownship_delta(id: u64, revision: u64) -> TrackDelta {
    let mut track = complete_track(id);
    track.ownship_shadow = true;
    TrackDelta::Updated(TrackSnapshotHandle::new(
        ProducerInstanceId::new(7),
        SnapshotRevision::new(revision),
        track,
    ))
}
