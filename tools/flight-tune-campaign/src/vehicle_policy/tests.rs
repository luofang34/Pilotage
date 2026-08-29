#![allow(clippy::expect_used, clippy::panic)]

use flight_tune::{PromotionPolicy, QualificationPolicy};

use super::{alia250_promotion_policy, alia250_qualification_policy};

/// Objectives the campaign is intended to hold a vehicle to, that nothing
/// measures yet.
///
/// These are not in either vehicle's bar, and they must not be: final
/// qualification requires every objective a bar names to be present in every
/// run, so naming one would fail the whole campaign on the name after every
/// run had been flown. They are recorded rather than deleted because the
/// intent is real — a wind-rejection bar and a hold-oscillation bar are things
/// a delivered calibration should have to clear.
///
/// The test below fails the moment one becomes measurable, which is the prompt
/// to put it in both vehicles' bars rather than to widen this list.
const OBJECTIVES_OWED_A_MEASUREMENT: [&str; 3] = [
    "wind.position_peak_m",
    "wind.position_p95_m",
    "wind.position_rms_m",
];

#[test]
fn an_objective_owed_a_measurement_is_not_in_any_bar_until_it_has_one() {
    // Two failures are possible here and both are wanted. If one of these
    // becomes measurable, the first assertion fires: add it to the bars. If one
    // reaches a bar while still unmeasurable, the second fires: a campaign
    // built on it would run to the end and prove nothing about the aircraft.
    for name in OBJECTIVES_OWED_A_MEASUREMENT {
        assert!(
            !pilotage_flight_quality::is_producible(name),
            "{name} is measurable now — put it in both vehicles' bars"
        );
        for (vehicle, qualification) in [
            ("alia250", alia250_qualification_policy()),
            ("x500", super::x500_qualification_policy()),
        ] {
            assert!(
                !qualification.objectives.contains(name),
                "{vehicle} names {name}, which nothing measures"
            );
        }
    }
}

#[test]
fn alia_policy_limits_are_finite_and_nonnegative() {
    let promotion = alia250_promotion_policy();
    let qualification = alia250_qualification_policy();

    promotion.validate().expect("validate promotion policy");
    assert!(
        promotion
            .objectives
            .iter()
            .eq(qualification.objectives.iter())
    );
    assert!(qualification.maximum_loss_confidence_upper.is_finite());
    assert!(qualification.maximum_loss_confidence_upper >= 0.0);
    assert!(qualification.maximum_p95_loss.is_finite());
    assert!(qualification.maximum_p95_loss >= 0.0);
    assert!(qualification.maximum_mean_control_effort.is_finite());
    assert!(qualification.maximum_mean_control_effort >= 0.0);
    let table = super::alia250_response_targets().expect("build response targets");
    assert!(
        table
            .targets
            .iter()
            .all(|row| row.limit.is_finite() && row.limit >= 0.0)
    );
}

/// Every objective a vehicle's bar names must be one the scoring layer can
/// measure.
///
/// Final qualification requires each named objective to be present in every
/// run. A bar naming a metric nothing produces does not fail open — it fails
/// the whole campaign on the name, after every run has been flown, and proves
/// nothing about the aircraft. This is the check a new vehicle gets for free:
/// state limits over the shared vocabulary, or do not qualify.
#[test]
fn every_vehicle_states_its_bar_over_metrics_the_scoring_layer_produces() {
    let vehicles: [(&str, QualificationPolicy, PromotionPolicy); 2] = [
        (
            "alia250",
            super::alia250_qualification_policy(),
            super::alia250_promotion_policy(),
        ),
        (
            "x500",
            super::x500_qualification_policy(),
            super::x500_promotion_policy(),
        ),
    ];
    for (vehicle, qualification, promotion) in vehicles {
        assert!(
            !qualification.objectives.is_empty(),
            "{vehicle} states no final bar"
        );
        for name in qualification.objectives.iter().chain(&promotion.objectives) {
            assert!(
                pilotage_flight_quality::is_producible(name),
                "{vehicle} bar names {name}, which nothing measures"
            );
        }
        // The two halves bound the same objectives: a metric held to an
        // absolute ceiling but not to a regression limit can drift toward the
        // ceiling one promotion at a time without any single step being
        // refused.
        assert_eq!(
            qualification.objectives.iter().collect::<Vec<_>>(),
            promotion.objectives.iter().collect::<Vec<_>>(),
            "{vehicle} bounds different objectives in its two halves"
        );
    }
}

/// A vehicle's absolute ceiling is looser than its per-promotion regression
/// limit, because they measure different things.
#[test]
fn each_absolute_ceiling_sits_above_its_regression_limit() {
    for (vehicle, model, table) in [
        (
            "alia250",
            crate::BenchVehicle::alia250(),
            super::alia250_response_targets().expect("build response targets"),
        ),
        (
            "x500",
            crate::BenchVehicle::x500(),
            super::x500_response_targets().expect("build response targets"),
        ),
    ] {
        let promotion_id = crate::bench_mission_revision_id(crate::BENCH_PROMOTION_TRIAL_ID, model)
            .expect("promotion mission identity");
        let final_id = crate::bench_mission_revision_id(crate::BENCH_FINAL_TRIAL_ID, model)
            .expect("final mission identity");
        for name in alia250_qualification_policy().objectives {
            let ceiling = table
                .target(&final_id, &name)
                .unwrap_or_else(|error| panic!("{vehicle} {name}: {error}"))
                .limit;
            let regression = table
                .target(&promotion_id, &name)
                .unwrap_or_else(|error| panic!("{vehicle} {name}: {error}"))
                .limit;
            assert!(
                ceiling > regression,
                "{vehicle} {name}: ceiling {ceiling} is not above its regression limit {regression}"
            );
        }
    }
}
