#![allow(clippy::expect_used, clippy::panic)]

use super::{
    check_vehicle_pairing, parse_area, parse_cruise_height, parse_execution_target,
    parse_ownship_source, parse_sim_camera_port, parse_survey_fov_deg, parse_survey_sidelap,
};
use crate::error::HostError;
use pilotage_mission::{ExecutionTarget, OwnshipSource};

#[test]
fn an_area_needs_three_lat_lon_pairs() {
    let polygon = parse_area("40.54,-74.46; 40.55,-74.46 ;40.55,-74.45").expect("three pairs");
    assert_eq!(
        polygon,
        vec![[40.54, -74.46], [40.55, -74.46], [40.55, -74.45]]
    );
    for value in [
        "40.54,-74.46;40.55,-74.46",
        "40.54;40.55;40.56",
        "a,b;c,d;e,f",
        "",
    ] {
        assert!(
            matches!(parse_area(value), Err(HostError::MissionArea { value: v }) if v == value),
            "{value:?} must be refused"
        );
    }
}

#[test]
fn survey_camera_parameters_are_range_checked() {
    assert!((parse_survey_fov_deg("69.6").expect("in range") - 69.6).abs() < 1e-12);
    for value in ["0", "180", "-5", "NaN", "wide"] {
        assert!(matches!(
            parse_survey_fov_deg(value),
            Err(HostError::MissionSurveyParameter {
                variable: "PILOTAGE_MISSION_SURVEY_FOV_DEG",
                ..
            })
        ));
    }
    assert!((parse_survey_sidelap("0.7").expect("in range") - 0.7).abs() < 1e-12);
    for value in ["0.96", "-0.1", "inf", "most"] {
        assert!(matches!(
            parse_survey_sidelap(value),
            Err(HostError::MissionSurveyParameter {
                variable: "PILOTAGE_MISSION_SURVEY_SIDELAP",
                ..
            })
        ));
    }
}

#[test]
fn ownship_source_and_target_words_parse_and_unknown_words_fail() {
    assert_eq!(
        parse_ownship_source("simulation-truth").expect("known"),
        OwnshipSource::SimulationTruth
    );
    assert_eq!(
        parse_ownship_source("vehicle-estimate").expect("known"),
        OwnshipSource::vehicle_estimate()
    );
    assert!(matches!(
        parse_ownship_source("truth").expect_err("unknown"),
        HostError::MissionOwnshipSource { value } if value == "truth"
    ));
    assert_eq!(
        parse_execution_target("simulator").expect("known"),
        ExecutionTarget::Simulator
    );
    assert_eq!(
        parse_execution_target("real-vehicle").expect("known"),
        ExecutionTarget::RealVehicle
    );
    assert!(matches!(
        parse_execution_target("drone").expect_err("unknown"),
        HostError::MissionTarget { value } if value == "drone"
    ));
}

#[test]
fn simulation_truth_on_a_real_vehicle_is_refused_at_startup() {
    assert!(matches!(
        check_vehicle_pairing(OwnshipSource::SimulationTruth, ExecutionTarget::RealVehicle),
        Err(HostError::MissionTruthOnRealVehicle)
    ));
    for (source, target) in [
        (OwnshipSource::SimulationTruth, ExecutionTarget::Simulator),
        (
            OwnshipSource::vehicle_estimate(),
            ExecutionTarget::Simulator,
        ),
        (
            OwnshipSource::vehicle_estimate(),
            ExecutionTarget::RealVehicle,
        ),
    ] {
        assert!(check_vehicle_pairing(source, target).is_ok());
    }
}

#[test]
fn a_negative_cruise_height_is_refused_with_the_value() {
    let error = parse_cruise_height("-5.0").expect_err("below launch elevation");
    assert!(
        matches!(error, HostError::MissionCruiseHeightRange { height } if height == -5.0),
        "got {error:?}"
    );
}

#[test]
fn a_non_finite_cruise_height_is_refused() {
    let error = parse_cruise_height("NaN").expect_err("not flyable");
    assert!(matches!(error, HostError::MissionCruiseHeightRange { .. }));
    let error = parse_cruise_height("inf").expect_err("not flyable");
    assert!(matches!(error, HostError::MissionCruiseHeightRange { .. }));
}

#[test]
fn a_sim_camera_port_must_name_a_port_a_producer_can_dial() {
    assert_eq!(parse_sim_camera_port("45991").expect("a port"), 45991);
    for value in ["0", "65536", "-1", "camera", ""] {
        let error = parse_sim_camera_port(value).expect_err("not a dialable port");
        assert!(
            matches!(&error, HostError::SimCameraPort { value: refused } if refused == value),
            "got {error:?}"
        );
    }
}

#[test]
fn zero_cruise_height_legitimately_disables_the_climb() {
    let height = parse_cruise_height("0").expect("zero is the no-climb configuration");
    assert_eq!(height, 0.0);
}
