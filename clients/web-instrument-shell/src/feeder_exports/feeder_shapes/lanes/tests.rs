#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_state::abi::v11::{CAPACITY, decode_state, encode_state};
use indicate_instrument_state::{
    AircraftState, NavFacility, NavFromTo, NavScale, NavSequencing, NavSource, PositionIntegrity,
    PositionSource, validate_state,
};
use pilotage_instrument_runtime::feeder::nav_display_state_with_rnp;

use super::{JsGuidanceIn, JsNavSnapshotIn};

/// Guidance as the page hands it over on a tracked leg.
fn tracked_leg() -> JsNavSnapshotIn {
    JsNavSnapshotIn {
        nav_guidance: JsGuidanceIn {
            to_ident: "WP2".to_owned(),
            from_ident: "WP1".to_owned(),
            course_rad: 0.4,
            lateral_deviation_m: -12.0,
            vertical_deviation_m: 3.0,
            distance_to_waypoint_m: 2_400.0,
            leg_index: 1,
            waypoint_count: 3,
            solution_quality: 0,
            sequencing: 0,
            to_from: 1,
            position_source: 1,
            position_integrity: 1,
            flight_phase: 2,
            rnp_m: 1_852.0,
        },
        age_ms: 120.0,
    }
}

#[test]
fn the_flight_plan_lane_declares_its_facility_and_sequencing_and_its_frame_decodes_valid() {
    frame_is_valid(tracked_leg(), NavSequencing::Sequencing, NavFromTo::To);
}

#[test]
fn a_hold_reaches_the_frame_as_hold_and_its_outbound_leg_as_from() {
    let mut outbound = tracked_leg();
    outbound.nav_guidance.sequencing = 1;
    outbound.nav_guidance.to_from = 2;
    frame_is_valid(outbound, NavSequencing::Hold, NavFromTo::From);
}

/// Feeds `guidance` through the lane and the feeder, encodes its nav group
/// in a state frame, and checks what decodes.
fn frame_is_valid(guidance: JsNavSnapshotIn, sequencing: NavSequencing, fromto: NavFromTo) {
    let (snapshot, rnp_m) = guidance.into_snapshot_and_rnp();
    let nav = nav_display_state_with_rnp(Some(&snapshot), rnp_m).expect("a nav group");
    let data = nav.data.expect("nav data");
    assert_eq!(
        data.facility,
        NavFacility::None,
        "the flight-plan lane names no facility"
    );
    assert_eq!(data.source, NavSource::Fms);
    assert_eq!(data.sequencing, sequencing);
    assert_eq!(data.fromto, fromto);
    let state = AircraftState {
        nav,
        ..AircraftState::default()
    };
    let mut frame = vec![0_u8; CAPACITY];
    let len = encode_state(&state, &mut frame).expect("encodes");
    let report = decode_state(frame.get(..len).expect("the written frame")).expect("decodes");
    let decoded = report.state.nav.data.expect("the nav group is on the wire");
    assert_eq!((decoded.sequencing, decoded.fromto), (sequencing, fromto));
    assert_eq!(
        validate_state(&report.state).nav,
        None,
        "an Unknown facility or sequencing would fail the whole nav group"
    );
}

/// The position source and integrity that reach the frame for the wire's
/// `position_source` and `position_integrity` codes, or `None` when the
/// lane gives no nav group.
fn position_on_the_frame(
    position_source: u32,
    position_integrity: u32,
) -> Option<(PositionSource, PositionIntegrity)> {
    let mut guidance = tracked_leg();
    guidance.nav_guidance.position_source = position_source;
    guidance.nav_guidance.position_integrity = position_integrity;
    let (snapshot, rnp_m) = guidance.into_snapshot_and_rnp();
    let nav = nav_display_state_with_rnp(Some(&snapshot), rnp_m)?;
    let state = AircraftState {
        nav,
        ..AircraftState::default()
    };
    let mut frame = vec![0_u8; CAPACITY];
    let len = encode_state(&state, &mut frame).expect("encodes");
    let report = decode_state(frame.get(..len).expect("the written frame")).expect("decodes");
    assert_eq!(validate_state(&report.state).nav, None, "a valid nav group");
    let data = report.state.nav.data?;
    Some((data.position_source, data.position_integrity))
}

#[test]
fn the_lane_names_the_position_it_rests_on() {
    use PositionIntegrity::{Degraded, Normal, SensorExcluded};
    let cases = [
        (1, 1, Some((PositionSource::Gps, Normal))),
        (2, 1, Some((PositionSource::Visual, Normal))),
        (3, 2, Some((PositionSource::Inertial, Degraded))),
        (3, 3, Some((PositionSource::Inertial, SensorExcluded))),
        (2, 3, Some((PositionSource::Visual, SensorExcluded))),
        (4, 2, Some((PositionSource::DeadReckoning, Degraded))),
        (5, 1, Some((PositionSource::Blended, Normal))),
        // No position or no integrity stated, or an unread code: the
        // guidance is removed, never drawn on an unnamed position.
        (0, 1, None),
        (6, 1, None),
        (1, 0, None),
        (1, 4, None),
    ];
    for (position_source, position_integrity, expected) in cases {
        assert_eq!(
            position_on_the_frame(position_source, position_integrity),
            expected,
            "position {position_source}, integrity {position_integrity}"
        );
    }
}

#[test]
fn flight_phase_switches_scale_and_tighter_rnp_narrows_deflection() {
    let cases = [
        (1, None, Some((NavScale::Enroute, 0.2_f32))),
        (2, None, Some((NavScale::Terminal, 0.8_f32))),
        (3, None, Some((NavScale::Approach, 2.0_f32))),
        (4, None, Some((NavScale::Terminal, 0.8_f32))),
        // Tighter procedure RNP (20 m) on terminal narrows ±50 m full-scale to ±20 m.
        (2, Some(20.0_f32), Some((NavScale::Terminal, 2.0_f32))),
        // Unstated or unknown flight phase removes the guidance display.
        (0, None, None),
        (5, None, None),
    ];
    for (flight_phase, rnp_override, expected) in cases {
        let mut guidance = tracked_leg();
        guidance.nav_guidance.lateral_deviation_m = -20.0;
        guidance.nav_guidance.flight_phase = flight_phase;
        guidance.nav_guidance.rnp_m = rnp_override.unwrap_or(f32::NAN);
        let (snapshot, rnp_m) = guidance.into_snapshot_and_rnp();
        let actual = nav_display_state_with_rnp(Some(&snapshot), rnp_m)
            .and_then(|stamped| stamped.data)
            .map(|data| (data.scale, data.cdi_dots));
        match (actual, expected) {
            (Some((scale, dots)), Some((expected_scale, expected_dots))) => {
                assert_eq!(scale, expected_scale, "phase {flight_phase}");
                assert!(
                    (dots - expected_dots).abs() < 1e-4,
                    "phase {flight_phase}, rnp {rnp_override:?}: got {dots}, expected {expected_dots}"
                );
            }
            (other, expected) => assert_eq!(other, expected, "phase {flight_phase}"),
        }
    }
}
