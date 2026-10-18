//! The velocity envelope against the Position-mode stick table.

use pilotage_mavlink::codec::ManualControlSticks;
use pilotage_protocol::{ReferenceFrame, VelocityIntent};

use super::{self as sticks, MAX_AIRSPEED_CHANGE_MPS, MAX_VERTICAL_MPS, max_turn_rate_rps};

fn velocity(vx: f32, vy: f32, vz: f32, yaw_rate: f32) -> VelocityIntent {
    VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy,
        vz,
        yaw_rate,
    }
}

#[test]
fn the_envelope_maps_onto_position_mode_sticks() {
    let neutral_with = |x: i16, y: i16, z: i16| ManualControlSticks {
        x,
        y,
        z,
        ..sticks::NEUTRAL
    };
    let cases = [
        (velocity(0.0, 0.0, 0.0, 0.0), sticks::NEUTRAL, false),
        (
            velocity(MAX_AIRSPEED_CHANGE_MPS, 0.0, 0.0, 0.0),
            neutral_with(0, 0, 1000),
            false,
        ),
        (
            velocity(-MAX_AIRSPEED_CHANGE_MPS, 0.0, 0.0, 0.0),
            neutral_with(0, 0, 0),
            false,
        ),
        // Full sink is the whole sink range; full climb is 2/3 of the
        // 3 m/s climb range, placed outside the 0.06 deadband.
        (
            velocity(0.0, 0.0, MAX_VERTICAL_MPS, 0.0),
            neutral_with(1000, 0, 500),
            false,
        ),
        (
            velocity(0.0, 0.0, -MAX_VERTICAL_MPS, 0.0),
            neutral_with(-687, 0, 500),
            false,
        ),
        (
            velocity(0.0, 0.0, 0.0, -max_turn_rate_rps()),
            neutral_with(0, -1000, 500),
            false,
        ),
        // A lateral demand cannot be flown and is dropped.
        (velocity(0.0, 1.0, 0.0, 0.0), sticks::NEUTRAL, true),
        // Beyond the envelope clamps to full stick.
        (
            velocity(2.0 * MAX_AIRSPEED_CHANGE_MPS, 0.0, 0.0, 0.0),
            neutral_with(0, 0, 1000),
            true,
        ),
    ];
    for (intent, expected, constrained) in cases {
        assert_eq!(
            sticks::sticks_from_velocity(&intent),
            (expected, constrained),
            "{intent:?}"
        );
    }
}

#[test]
fn half_the_turn_rate_banks_to_the_coordinated_turn_angle() {
    let (half, _) =
        sticks::sticks_from_velocity(&velocity(0.0, 0.0, 0.0, 0.5 * max_turn_rate_rps()));
    // tan(bank) halves, so the bank is atan(tan(50°) / 2) ≈ 30.8°.
    let bank_deg = (0.5 * 50.0_f32.to_radians().tan()).atan().to_degrees();
    assert_eq!(half.y, (bank_deg / 50.0 * 1000.0).round() as i16);
}
