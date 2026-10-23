//! The landmark fleet file plans, and its camera fault lands on its line.

use std::path::Path;

use super::plan::MissionFile;
use super::rehearse;

fn landmarks() -> MissionFile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(crate::cli::DEFAULT_MISSION_PLAN);
    MissionFile::load_blocking(&path).expect("the landmark fleet file loads")
}

#[test]
fn every_aircraft_of_the_landmark_fleet_plans_lines() {
    let file = landmarks();
    let proposals = rehearse::rehearse(&file);
    for aircraft in file.aircraft() {
        let proposal = proposals
            .get(&aircraft.lane())
            .unwrap_or_else(|| panic!("{} proposed nothing", aircraft.lane()));
        assert!(
            proposal
                .tasks
                .iter()
                .any(|t| t.task_id.starts_with("line-")),
            "{} plans no line",
            aircraft.lane()
        );
    }
}

#[test]
fn the_landmark_fault_sits_along_its_planned_line() {
    let file = landmarks();
    let fault = file.fault.as_ref().expect("the landmark fleet has a fault");
    let spec = rehearse::fault_spec(&file, fault).expect("the fault is placed");
    let boxes: Vec<&str> = spec.split(';').collect();
    assert_eq!(boxes.len(), 15);
    assert!(boxes.iter().all(|b| b.starts_with("dark@")));
    // Every box sits inside the survey area's bounds.
    let lats = file.objective.area.iter().map(|p| p[0]);
    let (south, north) = lats.fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    for b in boxes {
        let corners: Vec<f64> = b
            .trim_start_matches("dark@")
            .split(',')
            .map(|v| v.parse().expect("a number"))
            .collect();
        assert!(corners[0] > south && corners[2] < north, "{b}");
    }
}

#[test]
fn a_fault_on_a_line_the_aircraft_does_not_plan_is_refused_naming_its_lines() {
    let mut file = landmarks();
    let mut fault = file.fault.take().expect("a fault");
    fault.line = "line-99".to_owned();
    let error = rehearse::fault_spec(&file, &fault).expect_err("no such line");
    let text = error.to_string();
    assert!(
        text.contains("line-99") && text.contains("line-01"),
        "{text}"
    );
}

#[test]
fn a_fleet_file_that_names_a_lane_twice_is_refused() {
    let dir = std::env::temp_dir().join(format!("xtask-mission-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(crate::cli::DEFAULT_MISSION_PLAN);
    let text = std::fs::read_to_string(source).expect("the fleet file");
    let path = dir.join("twice.json");
    std::fs::write(&path, text.replace("\"MERCER\"", "\"BATTLEFIELD\"")).expect("written");
    let error = MissionFile::load_blocking(&path).expect_err("a repeated lane");
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        error.to_string().contains("BATTLEFIELD is used twice"),
        "{error}"
    );
}
