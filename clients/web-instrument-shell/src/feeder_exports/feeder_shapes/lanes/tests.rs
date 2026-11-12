#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_state::abi::v10::{CAPACITY, decode_state, encode_state};
use indicate_instrument_state::{
    AircraftState, NavFacility, NavFromTo, NavSequencing, validate_state,
};
use pilotage_instrument_runtime::feeder::nav_display_state;

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
        },
        age_ms: 120.0,
    }
}

#[test]
fn the_gps_lane_declares_its_facility_and_sequencing_and_its_frame_decodes_valid() {
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
/// in an ABI 10 frame, and checks what decodes.
fn frame_is_valid(guidance: JsNavSnapshotIn, sequencing: NavSequencing, fromto: NavFromTo) {
    let nav = nav_display_state(Some(&guidance.into_snapshot())).expect("a nav group");
    let data = nav.data.expect("nav data");
    assert_eq!(
        data.facility,
        NavFacility::None,
        "the GPS lane names no facility"
    );
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
