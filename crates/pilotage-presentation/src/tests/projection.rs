use surveillance_core::{
    FreshnessPolicy, ProducerInstanceId, ProjectionBounds, SnapshotRevision, TrackDelta,
    TrackSnapshotHandle, VelocityObservation,
};

use super::traffic::{complete_track, radio_timed};
use crate::PresentationAdapter;

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
    let at_report = reported
        .points
        .first()
        .expect("a track draws a point")
        .clone();
    assert!(
        !at_report.position_is_extrapolated,
        "a fresh report is not a guess"
    );

    // Five seconds later, with no new report.
    adapter.advance_time(5_000_000);
    let drawn = adapter.adapt();
    let later = drawn.points.first().expect("a track still draws a point");

    assert!(
        later.position_is_extrapolated,
        "the position must be marked as advanced"
    );
    assert!(
        later.coordinate.longitude_deg > at_report.coordinate.longitude_deg,
        "a track heading east must be drawn further east than it last reported"
    );
}
