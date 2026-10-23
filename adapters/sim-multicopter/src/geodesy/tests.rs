use super::{LocalFrame, quaternion_from_euler, rotate};

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn yaw_ninety_degrees_is_the_dji_quaternion() {
    let q = quaternion_from_euler(0.0, 0.0, 90.0_f64.to_radians());
    let half = 0.5_f64.sqrt();
    let expected = [half, 0.0, 0.0, half];
    for (got, want) in q.iter().zip(expected) {
        assert!(close(*got, want, 1e-12), "{q:?} != {expected:?}");
    }
}

#[test]
fn yaw_ninety_degrees_turns_body_forward_to_east() {
    let q = quaternion_from_euler(0.0, 0.0, 90.0_f64.to_radians());
    let world = rotate(q, [1.0, 0.0, 0.0]);
    assert!(close(world[0], 0.0, 1e-12));
    assert!(close(world[1], 1.0, 1e-12));
    assert!(close(world[2], 0.0, 1e-12));
}

#[test]
fn pitch_down_ninety_degrees_points_body_forward_down() {
    let q = quaternion_from_euler(0.0, -90.0_f64.to_radians(), 0.0);
    let world = rotate(q, [1.0, 0.0, 0.0]);
    assert!(close(world[0], 0.0, 1e-12));
    assert!(close(world[2], 1.0, 1e-12), "{world:?}");
}

#[test]
fn the_tangent_plane_round_trips() {
    let frame = LocalFrame::anchored_at(40.3317, -74.6745);
    let [lat, lon] = frame.lat_lon(250.0, -130.0);
    let [north, east] = frame.north_east(lat, lon);
    assert!(close(north, 250.0, 1e-6));
    assert!(close(east, -130.0, 1e-6));
    // One arc-minute of latitude is about 1852 m.
    let [north, _] = frame.north_east(40.3317 + 1.0 / 60.0, -74.6745);
    assert!(close(north, 1851.0, 5.0), "{north}");
}
