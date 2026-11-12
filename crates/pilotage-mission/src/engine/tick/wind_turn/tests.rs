use core::f64::consts::FRAC_PI_2;

use super::*;

const AIRSPEED_MPS: f64 = 60.0;

fn bank() -> f64 {
    22.0_f64.to_radians()
}

fn still_air_radius_m() -> f64 {
    AIRSPEED_MPS * AIRSPEED_MPS / (crate::body::SURFACE_GRAVITY_MPS2 * bank().tan())
}

#[test]
fn in_still_air_the_turn_starts_a_radius_times_the_half_turn_tangent_before() {
    for turn_deg in [30.0_f64, 90.0, 120.0, -90.0] {
        let lead = lead_m(
            0.3,
            0.3 + turn_deg.to_radians(),
            AIRSPEED_MPS,
            [0.0; 2],
            bank(),
        )
        .expect("a turn");
        let expected = still_air_radius_m() * (turn_deg.abs().to_radians() / 2.0).tan();
        assert!(
            (lead - expected).abs() < 0.01 * expected,
            "{turn_deg}°: {lead} m, {expected} m"
        );
    }
}

#[test]
fn a_turn_into_a_tailwind_starts_earlier_and_one_into_a_headwind_later() {
    let still = lead_m(0.0, FRAC_PI_2, AIRSPEED_MPS, [0.0; 2], bank()).expect("a turn");
    // North, then east: a wind toward the east is behind the next leg.
    let downwind = lead_m(0.0, FRAC_PI_2, AIRSPEED_MPS, [0.0, 12.0], bank()).expect("a turn");
    let upwind = lead_m(0.0, FRAC_PI_2, AIRSPEED_MPS, [0.0, -12.0], bank()).expect("a turn");
    assert!(downwind > still * 1.1, "{downwind} m against {still} m");
    assert!(upwind < still * 0.9, "{upwind} m against {still} m");
}

#[test]
fn no_turn_or_a_wind_beyond_the_airspeed_has_no_lead() {
    assert_eq!(lead_m(1.0, 1.0, AIRSPEED_MPS, [0.0; 2], bank()), None);
    assert_eq!(
        lead_m(0.0, FRAC_PI_2, AIRSPEED_MPS, [70.0, 0.0], bank()),
        None
    );
}
