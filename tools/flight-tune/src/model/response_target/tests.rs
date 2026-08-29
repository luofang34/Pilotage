#![allow(clippy::expect_used, clippy::panic)]

use pilotage_mission_core::MISSION_SCHEMA_VERSION;

use super::*;
use crate::{MissionReference, PhysicalUnit};

const OPERATOR_OBJECTIVES: [&str; 2] = ["control.effort_rms", "response.settling_time_s"];
const DIRECT_OBJECTIVES: [&str; 2] = ["angular.overshoot_fraction", "angular.settling_time_s"];

fn mission(id: &str, seed: u8) -> MissionReference {
    MissionReference {
        revision_id: id.to_owned(),
        schema_version: MISSION_SCHEMA_VERSION,
        content_digest: Digest::from_bytes([seed; 32]),
        max_samples: 64,
        sample_timeout_ns: 20_000_000,
    }
}

fn operator_scope(mission: &MissionReference) -> ResponseTargetScope {
    ResponseTargetScope {
        mission_revision_id: mission.revision_id.clone(),
        mission_content_digest: mission.content_digest,
        control_family: ControlFamily::OperatorVelocity,
        control_channel: ControlChannel::Roll,
        physical_target: PhysicalTarget {
            unit: PhysicalUnit::MetersPerSecond,
            value: 3.0,
        },
        envelope_digest: Digest::from_bytes([31; 32]),
        authority_band: Some(TargetAuthorityBand {
            minimum: 2.4,
            maximum: 3.0,
        }),
    }
}

fn direct_scope(mission: &MissionReference) -> ResponseTargetScope {
    ResponseTargetScope {
        mission_revision_id: mission.revision_id.clone(),
        mission_content_digest: mission.content_digest,
        control_family: ControlFamily::DirectAttitudeThrust,
        control_channel: ControlChannel::Roll,
        physical_target: PhysicalTarget {
            unit: PhysicalUnit::Radians,
            value: 0.174_532_925_199_432_95,
        },
        envelope_digest: Digest::from_bytes([32; 32]),
        authority_band: None,
    }
}

#[allow(dead_code)]
fn limits(names: [&str; 2]) -> Vec<(&str, TargetComparison, f64)> {
    names
        .into_iter()
        .map(|name| (name, TargetComparison::AtMost, 1.0))
        .collect()
}

fn operator_table() -> ResponseTargetTable {
    let promotion = mission("promotion", 3);
    ResponseTargetTable::new(operator_scope(&promotion).rows(limits(OPERATOR_OBJECTIVES)))
        .expect("the operator table is valid")
}

#[test]
fn a_table_states_one_row_for_one_objective_limit() {
    let table = operator_table();
    assert_eq!(table.targets.len(), OPERATOR_OBJECTIVES.len());
    let row = table
        .target("promotion", "response.settling_time_s")
        .expect("the row exists");
    assert_eq!(row.motion, ScenarioMotion::Linear);
    assert_eq!(row.control_family, ControlFamily::OperatorVelocity);
    assert!(row.holds(1.0));
    assert!(!row.holds(1.000_000_1));
}

#[test]
fn a_direct_attitude_objective_needs_a_direct_attitude_scope() {
    // The exact failure the scoped table exists to prevent: a velocity
    // scenario claiming a limit written for an attitude response.
    let promotion = mission("promotion", 3);
    let error =
        ResponseTargetTable::new(operator_scope(&promotion).rows(limits(DIRECT_OBJECTIVES)))
            .expect_err("a velocity scope cannot own an angular objective");
    assert!(
        error
            .to_string()
            .contains("does not belong to a operator_velocity linear scope"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_velocity_objective_needs_an_operator_scope() {
    // The mirror direction, and it is a refusal rather than an argument: the
    // operator family owns the step-response names, so an attitude scenario
    // cannot be held to a velocity limit any more than a velocity run can
    // answer an attitude one.
    let promotion = mission("promotion", 3);
    let error = ResponseTargetTable::new(direct_scope(&promotion).rows([(
        "response.settling_time_s",
        TargetComparison::AtMost,
        1.0,
    )]))
    .expect_err("a direct scope cannot own an operator objective");
    assert!(
        error
            .to_string()
            .contains("does not belong to a direct_attitude_thrust roll scope"),
        "unexpected error: {error}"
    );
    // The names are separate families too, so nothing one run produces can be
    // read as the other's measurement.
    for name in DIRECT_OBJECTIVES {
        assert!(!OPERATOR_OBJECTIVES.contains(&name));
    }
}

#[test]
fn an_unreserved_objective_belongs_to_every_scope() {
    // The rule reserves three prefixes and nothing else, so a vehicle that
    // measures something this repository does not know about still states a
    // valid bar on any family.
    let promotion = mission("promotion", 3);
    for scope in [operator_scope(&promotion), direct_scope(&promotion)] {
        ResponseTargetTable::new(scope.rows([
            ("control.effort_rms", TargetComparison::AtMost, 0.5),
            ("vendor.custom_metric", TargetComparison::AtMost, 1.0),
        ]))
        .expect("an unreserved name belongs to any scope");
    }
}

#[test]
fn a_collective_objective_needs_a_collective_scope() {
    let promotion = mission("promotion", 3);
    let error = ResponseTargetTable::new(direct_scope(&promotion).rows([(
        "collective.peak_response_mps2",
        TargetComparison::AtMost,
        4.0,
    )]))
    .expect_err("a roll scope cannot own a collective objective");
    assert!(
        error
            .to_string()
            .contains("does not belong to a direct_attitude_thrust roll scope"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_stated_motion_that_the_family_does_not_produce_is_refused() {
    let mut table = operator_table();
    table.targets[0].motion = ScenarioMotion::Roll;
    let error = table.validate().expect_err("the motion is substituted");
    assert!(
        error.to_string().contains("is not the one"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_unit_that_the_family_does_not_measure_is_refused() {
    let mut table = operator_table();
    for row in &mut table.targets {
        row.physical_target.unit = PhysicalUnit::Radians;
    }
    let error = table.validate().expect_err("the unit is substituted");
    assert!(
        error.to_string().contains("meters_per_second"),
        "unexpected error: {error}"
    );
}

#[test]
fn two_rows_for_one_scenario_cannot_state_different_scopes() {
    let mut table = operator_table();
    table.targets[1].envelope_digest = Digest::from_bytes([99; 32]);
    let error = table.validate().expect_err("the envelope is substituted");
    assert!(
        error.to_string().contains("different physical scopes"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_reordered_table_is_refused_and_has_another_identity() {
    let table = operator_table();
    let mut reordered = table.clone();
    reordered.targets.reverse();
    assert!(reordered.validate().is_err());
    assert_ne!(
        table.digest().expect("digest"),
        reordered.digest().expect("digest")
    );
}

#[test]
fn a_repeated_row_is_refused() {
    let mut table = operator_table();
    let repeat = table.targets[0].clone();
    table.targets.insert(1, repeat);
    let error = table.validate().expect_err("a row is repeated");
    assert!(
        error.to_string().contains("repeated or out of order"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_changed_limit_byte_changes_the_table_identity() {
    let table = operator_table();
    let mut tampered = table.clone();
    tampered.targets[0].limit = f64::from_bits(tampered.targets[0].limit.to_bits() ^ 1);
    assert_ne!(
        table.digest().expect("digest"),
        tampered.digest().expect("digest")
    );
}

#[test]
fn every_scoped_field_changes_the_table_identity() {
    let table = operator_table();
    let baseline = table.digest().expect("digest");
    let mut envelope = table.clone();
    envelope.targets[0].envelope_digest = Digest::from_bytes([77; 32]);
    let mut family = table.clone();
    family.targets[0].control_family = ControlFamily::DirectAttitudeThrust;
    let mut target = table.clone();
    target.targets[0].physical_target.value = 2.9;
    let mut comparison = table.clone();
    comparison.targets[0].comparison = TargetComparison::AtLeast;
    for changed in [envelope, family, target, comparison] {
        assert_ne!(baseline, changed.digest().expect("digest"));
    }
}

#[test]
fn an_authority_band_belongs_only_to_an_operator_scope() {
    let promotion = mission("promotion", 3);
    let mut scope = direct_scope(&promotion);
    scope.authority_band = Some(TargetAuthorityBand {
        minimum: 0.1,
        maximum: 0.2,
    });
    let error = ResponseTargetTable::new(scope.rows(limits(OPERATOR_OBJECTIVES)))
        .expect_err("a direct scope resolves its own target");
    assert!(
        error.to_string().contains("operator scope"),
        "unexpected error: {error}"
    );
}

#[test]
fn an_authority_band_sits_inside_its_own_physical_target() {
    // The envelope BOUNDS an operator output: the candidate curve shapes the
    // stick to at most the requested target, so a band above that target
    // states a check no run could ever meet.
    let promotion = mission("promotion", 3);
    let mut scope = operator_scope(&promotion);
    scope.authority_band = Some(TargetAuthorityBand {
        minimum: 2.4,
        maximum: 3.1,
    });
    let error = ResponseTargetTable::new(scope.rows(limits(OPERATOR_OBJECTIVES)))
        .expect_err("a band above its own target can never be met");
    assert!(
        error.to_string().contains("authority band"),
        "unexpected error: {error}"
    );

    // A band that reaches the target exactly is the widest one that can be
    // met, and it is accepted.
    scope.authority_band = Some(TargetAuthorityBand {
        minimum: 2.4,
        maximum: 3.0,
    });
    ResponseTargetTable::new(scope.rows(limits(OPERATOR_OBJECTIVES)))
        .expect("a band at its own target is valid");
}

#[test]
fn an_authority_band_refuses_a_target_a_larger_expo_lowered() {
    // A candidate that raises its expo resolves less physical speed for the
    // same stick, which improves every normalized response metric. The band
    // is the only measurement that sees it.
    let table = operator_table();
    assert!(table.authority_holds("promotion", Some(2.85)));
    assert!(!table.authority_holds("promotion", Some(1.90)));
    // A scenario that keeps no band keeps whatever its envelope resolves.
    assert!(table.authority_holds("other", Some(0.1)));
    // A banded scenario that states no resolved target has shown nothing.
    assert!(!table.authority_holds("promotion", None));
}

#[test]
fn a_banded_scenario_adds_its_authority_value_to_the_run_key_set() {
    let table = operator_table();
    let declared = OPERATOR_OBJECTIVES
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let banded = table.expected_objective_names("promotion", &declared);
    assert!(banded.contains(TARGET_AUTHORITY_OBJECTIVE));
    assert_eq!(banded.len(), declared.len() + 1);
    let unbanded = table.expected_objective_names("other", &declared);
    assert!(!unbanded.contains(TARGET_AUTHORITY_OBJECTIVE));
}

#[test]
fn the_authority_value_is_never_a_table_row() {
    let promotion = mission("promotion", 3);
    let error = ResponseTargetTable::new(operator_scope(&promotion).rows([(
        TARGET_AUTHORITY_OBJECTIVE,
        TargetComparison::AtLeast,
        2.4,
    )]))
    .expect_err("the band is not a one-sided paired limit");
    assert!(
        error
            .to_string()
            .contains("authority band, not a table row"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_zero_identity_or_a_negative_limit_is_refused() {
    let promotion = mission("promotion", 3);
    let mut scope = operator_scope(&promotion);
    scope.envelope_digest = Digest::from_bytes([0; 32]);
    assert!(ResponseTargetTable::new(scope.rows(limits(OPERATOR_OBJECTIVES))).is_err());

    let negative =
        operator_scope(&promotion).rows([("control.effort_rms", TargetComparison::AtMost, -1.0)]);
    assert!(ResponseTargetTable::new(negative).is_err());
}

#[test]
fn a_schema_change_is_refused() {
    let mut table = operator_table();
    table.schema_version = RESPONSE_TARGET_TABLE_SCHEMA_VERSION.wrapping_add(1);
    assert!(table.validate().is_err());
}

#[test]
fn an_empty_table_is_not_a_bar() {
    assert!(ResponseTargetTable::new(Vec::new()).is_err());
}

#[test]
fn a_generic_vehicle_table_passes_the_contract() {
    // Nothing in the contract names a vehicle, an objective list, or a limit
    // value. A second aircraft contributes numbers and its own scenario, and
    // the same validation accepts it.
    let promotion = mission("other-vehicle-promotion", 7);
    let scope = ResponseTargetScope {
        mission_revision_id: promotion.revision_id.clone(),
        mission_content_digest: promotion.content_digest,
        control_family: ControlFamily::OperatorVelocity,
        control_channel: ControlChannel::Yaw,
        physical_target: PhysicalTarget {
            unit: PhysicalUnit::RadiansPerSecond,
            value: 1.2,
        },
        envelope_digest: Digest::from_bytes([44; 32]),
        authority_band: None,
    };
    let table = ResponseTargetTable::new(scope.rows([
        ("vendor.custom_metric", TargetComparison::AtMost, 12.5),
        ("vendor.authority_margin", TargetComparison::AtLeast, 0.4),
    ]))
    .expect("a generic table is valid");
    assert_eq!(
        table
            .target("other-vehicle-promotion", "vendor.custom_metric")
            .expect("the row exists")
            .motion,
        ScenarioMotion::Yaw
    );
}

#[test]
fn each_family_and_channel_pair_derives_one_motion_and_one_unit() {
    let cases = [
        (
            ControlFamily::OperatorVelocity,
            ControlChannel::Roll,
            ScenarioMotion::Linear,
            PhysicalUnit::MetersPerSecond,
        ),
        (
            ControlFamily::OperatorVelocity,
            ControlChannel::Pitch,
            ScenarioMotion::Linear,
            PhysicalUnit::MetersPerSecond,
        ),
        (
            ControlFamily::OperatorVelocity,
            ControlChannel::Vertical,
            ScenarioMotion::Linear,
            PhysicalUnit::MetersPerSecond,
        ),
        (
            ControlFamily::OperatorVelocity,
            ControlChannel::Yaw,
            ScenarioMotion::Yaw,
            PhysicalUnit::RadiansPerSecond,
        ),
        (
            ControlFamily::DirectAttitudeThrust,
            ControlChannel::Roll,
            ScenarioMotion::Roll,
            PhysicalUnit::Radians,
        ),
        (
            ControlFamily::DirectAttitudeThrust,
            ControlChannel::Pitch,
            ScenarioMotion::Pitch,
            PhysicalUnit::Radians,
        ),
        (
            ControlFamily::DirectAttitudeThrust,
            ControlChannel::Yaw,
            ScenarioMotion::Yaw,
            PhysicalUnit::Radians,
        ),
        (
            ControlFamily::DirectAttitudeThrust,
            ControlChannel::Vertical,
            ScenarioMotion::Collective,
            PhysicalUnit::NormalizedCollectiveForce,
        ),
    ];
    for (family, channel, motion, unit) in cases {
        assert_eq!(ScenarioMotion::derive(family, channel), motion);
        assert_eq!(family.required_physics(channel).0, unit);
    }
    // The operator and the direct family share the yaw motion and are still
    // separate scopes, because they measure it in different units.
    assert_eq!(
        ScenarioMotion::derive(ControlFamily::OperatorVelocity, ControlChannel::Yaw),
        ScenarioMotion::derive(ControlFamily::DirectAttitudeThrust, ControlChannel::Yaw)
    );
    assert_ne!(
        ControlFamily::OperatorVelocity
            .required_physics(ControlChannel::Yaw)
            .0,
        ControlFamily::DirectAttitudeThrust
            .required_physics(ControlChannel::Yaw)
            .0
    );
}
