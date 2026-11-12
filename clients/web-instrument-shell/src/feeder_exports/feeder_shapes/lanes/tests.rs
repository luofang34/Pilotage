#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_state::abi::v10::{CAPACITY, decode_state, encode_state};
use indicate_instrument_state::{AircraftState, NavFacility, NavSequencing, validate_state};
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
        },
        age_ms: 120.0,
    }
}

#[test]
fn the_gps_lane_declares_its_facility_and_sequencing_and_its_frame_decodes_valid() {
    let nav = nav_display_state(Some(&tracked_leg().into_snapshot())).expect("a nav group");
    let data = nav.data.expect("nav data");
    assert_eq!(
        data.facility,
        NavFacility::None,
        "the GPS lane names no facility"
    );
    assert_eq!(data.sequencing, NavSequencing::Sequencing);
    let state = AircraftState {
        nav,
        ..AircraftState::default()
    };
    let mut frame = vec![0_u8; CAPACITY];
    let len = encode_state(&state, &mut frame).expect("encodes");
    let report = decode_state(frame.get(..len).expect("the written frame")).expect("decodes");
    assert!(
        report.state.nav.data.is_some(),
        "the nav group is on the wire"
    );
    assert_eq!(
        validate_state(&report.state).nav,
        None,
        "an Unknown facility or sequencing would fail the whole nav group"
    );
}
