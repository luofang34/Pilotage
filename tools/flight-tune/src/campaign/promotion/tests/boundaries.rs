use super::super::calculate;
use super::{MetricPoint, evidence, expected_pairs, next_up, plan, receipt, stage};
use crate::PromotionDecision;

#[test]
fn valid_complete_comparison_promotes_the_frozen_candidate() {
    let stage = stage();
    let evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
    let calculation = calculate(&stage, plan(), &evidence.baseline, &evidence.frozen)
        .expect("calculate promotion");

    assert!(matches!(
        calculation.selection.decision,
        PromotionDecision::Promoted { .. }
    ));
    assert_eq!(
        calculation.selection.selected_candidate,
        Some(plan().frozen_candidate_digest)
    );
    assert!(calculation.comparison.all_passed());
    assert_eq!(
        calculation.comparison.scenarios["promotion-calm"]
            .objectives
            .len(),
        3
    );
}

#[test]
fn absolute_loss_relative_loss_and_effort_reject_independently() {
    let cases = [
        (0.25, 0.0, 0.1, MetricPoint::passing()),
        (0.0, 0.25, 0.1, MetricPoint::passing()),
        (
            0.1,
            0.05,
            0.1,
            MetricPoint {
                effort: 0.45,
                ..MetricPoint::passing()
            },
        ),
    ];
    for (minimum, relative, effort, frozen) in cases {
        let mut stage = stage();
        stage.promotion.minimum_loss_improvement = minimum;
        stage.promotion.minimum_relative_loss_improvement = relative;
        stage.promotion.maximum_control_effort_increase = effort;
        let evidence = evidence(&stage, MetricPoint::baseline(), frozen);
        let calculation = calculate(&stage, plan(), &evidence.baseline, &evidence.frozen)
            .expect("calculate rejection");
        assert_rejected(&calculation.selection.decision);
        assert_eq!(
            calculation.selection.selected_candidate,
            Some(plan().initial_candidate_digest)
        );
    }
}

#[test]
fn aggregate_loss_cannot_hide_one_named_objective_regression() {
    let stage = stage();
    let frozen = MetricPoint {
        tracking: 0.4,
        ..MetricPoint::passing()
    };
    let evidence = evidence(&stage, MetricPoint::baseline(), frozen);
    let calculation = calculate(&stage, plan(), &evidence.baseline, &evidence.frozen)
        .expect("calculate objective rejection");

    assert!(calculation.comparison.loss_passed);
    let results = &calculation.comparison.scenarios["promotion-calm"].objectives;
    assert!(!results["tracking"].passed);
    assert!(results["settling"].passed);
    assert!(results["overshoot"].passed);
    assert_rejected(&calculation.selection.decision);
}

#[test]
fn loss_limits_pass_at_equality_and_fail_at_the_next_value() {
    for relative in [false, true] {
        let mut stage = stage();
        stage.promotion.minimum_loss_improvement = if relative { 0.0 } else { 0.125 };
        stage.promotion.minimum_relative_loss_improvement = if relative { 0.5 } else { 0.0 };
        let baseline = MetricPoint {
            loss: 0.25,
            ..MetricPoint::baseline()
        };
        let equal = MetricPoint {
            loss: 0.125,
            ..MetricPoint::passing()
        };
        let equal_evidence = evidence(&stage, baseline, equal);
        let calculation = calculate(
            &stage,
            plan(),
            &equal_evidence.baseline,
            &equal_evidence.frozen,
        )
        .expect("calculate equality");
        assert!(calculation.comparison.loss_passed);

        let over = MetricPoint {
            loss: next_up(0.125),
            ..equal
        };
        let over_evidence = evidence(&stage, baseline, over);
        let calculation = calculate(
            &stage,
            plan(),
            &over_evidence.baseline,
            &over_evidence.frozen,
        )
        .expect("calculate adjacent failure");
        assert!(!calculation.comparison.loss_passed);
    }
}

#[test]
fn effort_limit_passes_at_equality_and_fails_at_the_next_value() {
    let mut stage = stage();
    stage.promotion.maximum_control_effort_increase = 0.125;
    let baseline = MetricPoint {
        effort: 0.0,
        ..MetricPoint::baseline()
    };
    let equal = MetricPoint {
        effort: 0.125,
        ..MetricPoint::passing()
    };
    let equal_evidence = evidence(&stage, baseline, equal);
    let calculation = calculate(
        &stage,
        plan(),
        &equal_evidence.baseline,
        &equal_evidence.frozen,
    )
    .expect("calculate equality");
    assert!(calculation.comparison.control_effort_passed);

    let over = MetricPoint {
        effort: next_up(0.125),
        ..equal
    };
    let over_evidence = evidence(&stage, baseline, over);
    let calculation = calculate(
        &stage,
        plan(),
        &over_evidence.baseline,
        &over_evidence.frozen,
    )
    .expect("calculate adjacent failure");
    assert!(!calculation.comparison.control_effort_passed);
}

#[test]
fn each_objective_limit_has_an_inclusive_float_boundary() {
    for objective in ["tracking", "settling", "overshoot"] {
        let mut stage = stage();
        stage.response_targets = crate::model::response_target::fixture::covering(&[
            (
                &[super::super::tests::scenario("promotion-calm", 12)],
                &super::super::tests::objective_limits(0.125),
            ),
            (
                &[super::super::tests::scenario("final-calm", 13)],
                &super::super::tests::objective_limits(1.0),
            ),
        ]);
        let baseline = MetricPoint {
            tracking: 0.0,
            settling: 0.0,
            overshoot: 0.0,
            ..MetricPoint::baseline()
        };
        let equal = MetricPoint {
            tracking: 0.125,
            settling: 0.125,
            overshoot: 0.125,
            ..MetricPoint::passing()
        };
        let equal_evidence = evidence(&stage, baseline, equal);
        let calculation = calculate(
            &stage,
            plan(),
            &equal_evidence.baseline,
            &equal_evidence.frozen,
        )
        .expect("calculate objective equality");
        assert!(calculation.comparison.scenarios["promotion-calm"].objectives[objective].passed);

        let mut over = equal;
        match objective {
            "tracking" => over.tracking = next_up(0.125),
            "settling" => over.settling = next_up(0.125),
            "overshoot" => over.overshoot = next_up(0.125),
            _ => panic!("unknown objective"),
        }
        let over_evidence = evidence(&stage, baseline, over);
        let calculation = calculate(
            &stage,
            plan(),
            &over_evidence.baseline,
            &over_evidence.frozen,
        )
        .expect("calculate objective adjacent failure");
        assert!(!calculation.comparison.scenarios["promotion-calm"].objectives[objective].passed);
    }
}

#[test]
fn comparison_validation_rejects_non_finite_or_changed_results() {
    let stage = stage();
    let evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
    let calculation = calculate(&stage, plan(), &evidence.baseline, &evidence.frozen)
        .expect("calculate promotion");
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut changed = calculation.comparison.clone();
        changed.loss.mean = value;
        assert!(changed.validate_for(&stage).is_err());
    }
    let mut changed = calculation.comparison;
    changed
        .scenarios
        .get_mut("promotion-calm")
        .expect("promotion scenario")
        .objectives
        .get_mut("tracking")
        .expect("tracking")
        .passed = false;
    assert!(changed.validate_for(&stage).is_err());
}

#[test]
fn nonzero_variance_student_t_result_has_a_fixed_vector() {
    let stage = stage();
    let pairs = expected_pairs(&stage);
    let baseline = pairs
        .iter()
        .map(|pair| {
            receipt(
                &pair.baseline,
                MetricPoint::baseline(),
                MetricPoint::baseline().objectives(),
            )
        })
        .collect::<Vec<_>>();
    let frozen = pairs
        .iter()
        .zip([0.7, 0.8, 0.9])
        .map(|(pair, loss)| {
            let point = MetricPoint {
                loss,
                ..MetricPoint::passing()
            };
            receipt(&pair.frozen, point, point.objectives())
        })
        .collect::<Vec<_>>();

    let calculation =
        calculate(&stage, plan(), &baseline, &frozen).expect("calculate nonzero variance vector");

    assert_eq!(
        calculation.comparison.loss.mean.to_bits(),
        0xbfc9_9999_9999_9999
    );
    assert_eq!(
        calculation.comparison.loss.upper_95.to_bits(),
        0x3fa8_cc51_58fd_753c
    );
}

fn assert_rejected(decision: &PromotionDecision) {
    assert!(matches!(
        decision,
        PromotionDecision::RejectedNoImprovement { .. }
    ));
}
