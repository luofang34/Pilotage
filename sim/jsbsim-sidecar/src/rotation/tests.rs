//! The attitude conventions against hand-checked cases.

use super::Euler;

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-12)
}

#[test]
fn yawing_east_turns_north_onto_the_left_wing() {
    let east = Euler {
        roll: 0.0,
        pitch: 0.0,
        yaw: std::f64::consts::FRAC_PI_2,
    };
    // Facing east, north is to the left (negative body y).
    assert!(close(east.ned_to_body([1.0, 0.0, 0.0]), [0.0, -1.0, 0.0]));
    assert!(close(east.body_to_ned([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
}

#[test]
fn pitching_up_puts_gravity_behind_the_nose() {
    let up = Euler {
        roll: 0.0,
        pitch: 0.3,
        yaw: 0.0,
    };
    let g = up.ned_to_body([0.0, 0.0, 1.0]);
    assert!(g[0] < 0.0 && g[2] > 0.0, "{g:?}");
}

#[test]
fn the_quaternion_and_the_matrix_rotate_alike() {
    let attitude = Euler {
        roll: 0.2,
        pitch: -0.4,
        yaw: 2.5,
    };
    let q = attitude.quaternion();
    let v = [0.3, -1.2, 0.7];
    // q v q* for a unit quaternion.
    let (w, x, y, z) = (q.w, q.x, q.y, q.z);
    let rotated = [
        (1.0 - 2.0 * (y * y + z * z)) * v[0]
            + 2.0 * (x * y - w * z) * v[1]
            + 2.0 * (x * z + w * y) * v[2],
        2.0 * (x * y + w * z) * v[0]
            + (1.0 - 2.0 * (x * x + z * z)) * v[1]
            + 2.0 * (y * z - w * x) * v[2],
        2.0 * (x * z - w * y) * v[0]
            + 2.0 * (y * z + w * x) * v[1]
            + (1.0 - 2.0 * (x * x + y * y)) * v[2],
    ];
    assert!(close(rotated, attitude.body_to_ned(v)));
    assert!(close(attitude.ned_to_body(attitude.body_to_ned(v)), v));
    assert!(((w * w + x * x + y * y + z * z) - 1.0).abs() < 1e-12);
}
