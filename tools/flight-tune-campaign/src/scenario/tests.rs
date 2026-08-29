#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use pilotage_trial::{CommandLossPolicy, ControlFamily, DelayJitter};

use super::*;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/alia250-xplane")
}

fn loaded() -> LoadedMatrix {
    LoadedMatrix::load_blocking(&ALIA250_MATRIX, &corpus()).expect("the checked-in corpus loads")
}

#[test]
fn the_declaration_derives_its_own_file_count() {
    // The count follows from the two pairing rules rather than from a number
    // anyone recorded: fifteen stimuli calm plus eleven factors on two family
    // representatives, in each of three partitions.
    let matrix = ALIA250_MATRIX;
    assert_eq!(matrix.stimuli.len(), 15);
    assert_eq!(matrix.conditions.len(), 12);
    assert_eq!(matrix.family_representatives.len(), 2);
    let per_partition = 15 + 11 * 2;
    assert_eq!(matrix.expected_cell_count(), 3 * per_partition);
    assert_eq!(matrix.expected_document_count(), 2 * 3 * per_partition);
    assert_eq!(matrix.cells().len(), matrix.expected_cell_count());
}

#[test]
fn the_checked_in_corpus_matches_its_declaration_exactly() {
    let matrix = loaded();
    assert_eq!(matrix.cells().len(), ALIA250_MATRIX.expected_cell_count());
}

#[test]
fn every_condition_artifact_decodes_at_the_current_schema() {
    // The decode is the point: an artifact at an older schema cannot pass
    // scenario preparation, so a corpus that only says it is current is a
    // corpus a campaign fails on.
    for loaded in loaded().cells() {
        assert_eq!(
            loaded.condition.schema_version,
            flight_tune::CONDITION_SET_SCHEMA_VERSION,
            "{} declares another schema",
            loaded.condition.id
        );
        loaded
            .condition
            .validate()
            .expect("a generated condition is valid");
    }
}

#[test]
fn every_artifact_is_its_own_canonical_encoding() {
    // load_blocking refuses an artifact whose bytes are not canonical, so
    // reaching this point is the assertion. Restating it here names what the
    // corpus guarantees rather than leaving it implied by a loader.
    let matrix = loaded();
    for cell in matrix.cells() {
        let bytes = std::fs::read(condition_path(&corpus(), &cell.cell)).expect("condition bytes");
        assert_eq!(
            bytes,
            cell.condition
                .to_canonical_json()
                .expect("canonical condition")
        );
    }
}

#[test]
fn every_scenario_names_the_condition_identity_its_artifact_produces() {
    // A scenario carrying a stale condition digest would run one disturbance
    // and name another. The loader recomputes the identity from the artifact
    // bytes rather than reading the scenario's claim.
    for cell in loaded().cells() {
        let declared = cell
            .condition
            .canonical_digest()
            .expect("condition identity");
        let applied = cell
            .scenario
            .phases
            .iter()
            .find_map(|phase| match &phase.action {
                pilotage_trial::PhaseAction::ApplyConditions { condition_set } => {
                    Some(condition_set.digest)
                }
                _ => None,
            })
            .expect("the scenario applies a condition");
        assert_eq!(applied, declared);
    }
}

#[test]
fn every_declared_factor_is_covered_by_an_executable_value() {
    let matrix = loaded();
    let report = matrix
        .coverage(&ALIA250_MATRIX)
        .expect("every factor is covered");
    assert_eq!(report.cells, ALIA250_MATRIX.expected_cell_count());
    for condition in ALIA250_MATRIX.conditions {
        let partitions = report
            .covered
            .get(condition.id)
            .unwrap_or_else(|| panic!("{} has no coverage entry", condition.id));
        assert_eq!(
            partitions,
            &["training", "promotion", "final"],
            "{} is not covered in every partition",
            condition.id
        );
    }
}

#[test]
fn a_calm_artifact_cannot_cover_an_uncertainty_factor() {
    // The rule the coverage half of the condition contract exists for: a cell
    // named after a factor whose artifact requests nothing covers nothing.
    for cell in loaded().cells() {
        if cell.cell.condition.id != "calm" {
            continue;
        }
        let condition = &cell.condition;
        assert_eq!(condition.actuator.authority_scale_basis_points, 10_000);
        assert!(condition.sensor.is_nominal());
        assert_eq!(condition.timing.estimate_delay_ns, 0);
        assert!(matches!(condition.timing.update_jitter, DelayJitter::None));
        assert!(matches!(
            condition.actuator.command_loss,
            CommandLossPolicy::None {}
        ));
        assert!(
            condition
                .controller_initialization
                .has_nominal_hover_thrust_force()
        );
        assert!(condition.wind.gusts.is_empty());
        assert_eq!(condition.wind.steady.speed_mps, 0.0);
    }
}

#[test]
fn each_uncertainty_artifact_carries_its_exact_executable_value() {
    let matrix = loaded();
    let value = |condition_id: &str, extract: fn(&pilotage_trial::ConditionSet) -> String| {
        matrix
            .cells()
            .iter()
            .find(|cell| cell.cell.condition.id == condition_id)
            .map(|cell| extract(&cell.condition))
            .unwrap_or_default()
    };
    assert_eq!(
        value("authority-low", |condition| condition
            .actuator
            .authority_scale_basis_points
            .to_string()),
        "8000"
    );
    assert_eq!(
        value("authority-high", |condition| condition
            .actuator
            .authority_scale_basis_points
            .to_string()),
        "12000"
    );
    assert_eq!(
        value("hover-trim-low", |condition| condition
            .controller_initialization
            .hover_thrust_force
            .scale_basis_points()
            .to_string()),
        "9000"
    );
    assert_eq!(
        value("added-delay", |condition| condition
            .timing
            .estimate_delay_ns
            .to_string()),
        "30000000"
    );
    assert_eq!(
        value("sensor-noise", |condition| condition
            .sensor
            .noise_lanes()
            .len()
            .to_string()),
        "6"
    );
    assert_eq!(
        value("crosswind", |condition| condition
            .wind
            .steady
            .speed_mps
            .to_string()),
        "5"
    );
}

#[test]
fn no_two_cells_run_the_same_seed() {
    let mut seeds = std::collections::BTreeSet::new();
    for cell in loaded().cells() {
        assert!(
            seeds.insert(cell.condition.seed),
            "{} repeats a seed",
            cell.scenario.id
        );
    }
}

#[test]
fn every_control_family_reaches_every_uncertainty_factor() {
    // A factor exercised on one family only would leave the other family's
    // response unmeasured under it, and the coverage report would still read
    // complete.
    let matrix = loaded();
    for condition in ALIA250_MATRIX.uncertainty_conditions() {
        for family in [
            ControlFamily::OperatorVelocity,
            ControlFamily::DirectAttitudeThrust,
        ] {
            assert!(
                matrix.cells().iter().any(|cell| {
                    cell.cell.condition.id == condition.id && cell.cell.stimulus.family == family
                }),
                "{} never meets {}",
                condition.id,
                family.as_str()
            );
        }
    }
}

#[test]
fn a_missing_cell_is_refused() {
    let scratch = std::env::temp_dir().join(format!("matrix-missing-{}", std::process::id()));
    copy_corpus(&scratch);
    let cell = ALIA250_MATRIX.cells()[0];
    std::fs::remove_file(scenario_path(&scratch, &cell)).expect("remove one scenario");
    let error = LoadedMatrix::load_blocking(&ALIA250_MATRIX, &scratch)
        .expect_err("a corpus missing a declared cell is not the declared matrix");
    assert!(
        error.to_string().contains("cannot read"),
        "unexpected error: {error}"
    );
    std::fs::remove_dir_all(&scratch).ok();
}

#[test]
fn an_orphan_artifact_is_refused() {
    let scratch = std::env::temp_dir().join(format!("matrix-orphan-{}", std::process::id()));
    copy_corpus(&scratch);
    let cell = ALIA250_MATRIX.cells()[0];
    let source = scenario_path(&scratch, &cell);
    std::fs::copy(&source, scratch.join("scenarios/orphan.json")).expect("write an orphan");
    let error = LoadedMatrix::load_blocking(&ALIA250_MATRIX, &scratch)
        .expect_err("a file the declaration does not name is an orphan");
    assert!(
        error.to_string().contains("orphan"),
        "unexpected error: {error}"
    );
    std::fs::remove_dir_all(&scratch).ok();
}

#[test]
fn a_hand_edited_artifact_is_refused() {
    let scratch = std::env::temp_dir().join(format!("matrix-edited-{}", std::process::id()));
    copy_corpus(&scratch);
    let cell = ALIA250_MATRIX.cells()[0];
    let path = condition_path(&scratch, &cell);
    let mut bytes = std::fs::read(&path).expect("condition bytes");
    bytes.push(b'\n');
    std::fs::write(&path, &bytes).expect("append a newline");
    let error = LoadedMatrix::load_blocking(&ALIA250_MATRIX, &scratch)
        .expect_err("a non-canonical artifact has two identities for one meaning");
    assert!(
        error.to_string().contains("canonical"),
        "unexpected error: {error}"
    );
    std::fs::remove_dir_all(&scratch).ok();
}

fn copy_corpus(target: &std::path::Path) {
    std::fs::remove_dir_all(target).ok();
    for directory in ["conditions", "scenarios"] {
        std::fs::create_dir_all(target.join(directory)).expect("create the scratch directory");
        for entry in std::fs::read_dir(corpus().join(directory)).expect("read the corpus") {
            let path = entry.expect("read one corpus entry").path();
            let name = path.file_name().expect("a file name");
            std::fs::copy(&path, target.join(directory).join(name)).expect("copy one artifact");
        }
    }
}

/// The Alia direct-response gates, stated once and checked where they decide.
///
/// Each case is a measured value the issue names and the verdict the frozen
/// table gives it. The measurements themselves are proven in
/// `pilotage-flight-quality`; what these prove is that the bar written for one
/// scenario is the bar that scenario's result meets.
#[test]
fn the_alia_direct_gates_refuse_the_results_the_issue_names() {
    let table = alia250_matrix_response_targets(&corpus()).expect("the frozen bar is valid");
    let mission = |stimulus: &str, condition: &str, partition: MatrixPartition| -> String {
        let cell = loaded()
            .cells()
            .iter()
            .find(|cell| {
                cell.cell.partition == partition
                    && cell.cell.stimulus.id == stimulus
                    && cell.cell.condition.id == condition
            })
            .map(|cell| matrix_mission(cell, 1_200).expect("mission identity"))
            .expect("the cell exists");
        cell.revision_id
    };

    let roll = mission(
        "roll-step-10deg",
        "calm",
        MatrixPartition::FinalQualification,
    );
    let settling = table
        .target(&roll, "angular.settling_time_s")
        .expect("the ten degree roll step states a settling limit");
    assert!(settling.holds(1.0), "one second settles inside the limit");
    assert!(!settling.holds(1.01), "1.01 seconds is over the limit");

    let overshoot = table
        .target(&roll, "angular.overshoot_fraction")
        .expect("the ten degree roll step states an overshoot limit");
    assert!(
        overshoot.holds(0.30),
        "the compliance maximum is thirty percent"
    );
    assert!(
        !overshoot.holds(0.31),
        "above thirty percent fails compliance"
    );

    let promotion_roll = mission("roll-step-10deg", "calm", MatrixPartition::Promotion);
    let target = table
        .target(&promotion_roll, "angular.overshoot_fraction")
        .expect("the campaign target");
    assert!(target.holds(0.05), "the campaign target is five percent");
    assert!(!target.holds(0.051), "above five percent misses the target");

    let yaw = mission(
        "yaw-step-10deg",
        "calm",
        MatrixPartition::FinalQualification,
    );
    let yaw_overshoot = table
        .target(&yaw, "angular.overshoot_fraction")
        .expect("the yaw step states an overshoot limit");
    assert!(yaw_overshoot.holds(0.10));
    assert!(!yaw_overshoot.holds(0.101));
    let yaw_settling = table
        .target(&yaw, "angular.settling_time_s")
        .expect("the yaw step states a settling limit");
    assert!(yaw_settling.holds(2.5));
    assert!(!yaw_settling.holds(2.51));

    let return_zero = mission(
        "roll-return-zero",
        "calm",
        MatrixPartition::FinalQualification,
    );
    let peak = table
        .target(&return_zero, "angular_release.opposite_return_peak_rad")
        .expect("the return states an opposite peak limit");
    assert!(peak.holds(0.5 * DEGREE_RAD));
    assert!(!peak.holds(0.51 * DEGREE_RAD));
    let rate = table
        .target(&return_zero, "angular_release.final_body_rate_rms_rps")
        .expect("the return states a final body-rate limit");
    assert!(rate.holds(0.5 * DEGREE_RAD));
    assert!(!rate.holds(0.51 * DEGREE_RAD));
}

const DEGREE_RAD: f64 = 0.017_453_292_519_943_295;

#[test]
fn every_scoped_scenario_states_only_the_objectives_its_family_produces() {
    // A collective scenario has no attitude response and an operator scenario
    // has no angular one. The table states what each scope measures, so no
    // limit written for one family can decide a run of another.
    let table = alia250_matrix_response_targets(&corpus()).expect("the frozen bar is valid");
    for row in &table.targets {
        assert!(
            flight_tune::is_admissible(&row.objective, row.control_family, row.motion),
            "{} is scoped to a {} {} it cannot measure",
            row.objective,
            row.control_family.as_str(),
            row.motion.as_str()
        );
    }
    let families: std::collections::BTreeSet<&str> = table
        .targets
        .iter()
        .map(|row| row.control_family.as_str())
        .collect();
    assert_eq!(families.len(), 2, "the bar covers both control families");
    let angular = table
        .targets
        .iter()
        .filter(|row| row.objective.starts_with("angular."))
        .count();
    let collective = table
        .targets
        .iter()
        .filter(|row| row.objective.starts_with("collective."))
        .count();
    let response = table
        .targets
        .iter()
        .filter(|row| row.objective.starts_with("response."))
        .count();
    assert!(angular > 0 && collective > 0 && response > 0);
}

#[test]
fn only_an_operator_scope_keeps_an_authority_band() {
    let table = alia250_matrix_response_targets(&corpus()).expect("the frozen bar is valid");
    for row in &table.targets {
        let operator = row.control_family == flight_tune::ControlFamily::OperatorVelocity;
        assert_eq!(
            row.authority_band.is_some(),
            operator,
            "{} keeps the wrong authority",
            row.mission_revision_id
        );
    }
}

#[test]
fn a_changed_frozen_limit_changes_the_bar_identity() {
    let table = alia250_matrix_response_targets(&corpus()).expect("the frozen bar is valid");
    let baseline = table.digest().expect("bar identity");
    let mut loosened = table.clone();
    loosened.targets[0].limit = f64::from_bits(loosened.targets[0].limit.to_bits() ^ 1);
    assert_ne!(baseline, loosened.digest().expect("bar identity"));
}

/// The frozen bar states one row for each objective each scoped scenario can
/// answer, and the count follows from the declaration rather than from a
/// number anyone recorded.
#[test]
fn the_frozen_bar_states_one_row_for_each_scoped_objective() {
    let table = alia250_matrix_response_targets(&corpus()).expect("the frozen bar is valid");
    let matrix = loaded();
    let mut expected = 0_usize;
    for cell in matrix.cells() {
        if cell.cell.partition == MatrixPartition::Training {
            continue;
        }
        expected += match (cell.cell.stimulus.family, cell.cell.stimulus.channel) {
            // Two shared control objectives, plus what the scope measures:
            // four angular, two collective, or two operator step values.
            (ControlFamily::DirectAttitudeThrust, pilotage_trial::ControlChannel::Vertical) => 4,
            (ControlFamily::DirectAttitudeThrust, _) => 6,
            (ControlFamily::OperatorVelocity, _) => 4,
        };
    }
    assert_eq!(table.targets.len(), expected);
    // Two partitions of thirty-seven cells: twenty direct angular, two
    // collective, fifteen operator.
    assert_eq!(expected, 2 * (20 * 6 + 2 * 4 + 15 * 4));
}
