use super::{GateContext, HardGate, HardGateOutcome, HardGateReport};

#[test]
fn one_hard_failure_keeps_the_report_failed() {
    let report = HardGateReport::new(vec![
        HardGateOutcome::pass(HardGate::TrialIdentity, GateContext::Trial),
        HardGateOutcome::fail(
            HardGate::RecoveryDeadline,
            GateContext::Phase { phase_index: 4 },
        ),
    ]);

    assert!(!report.passed());
    assert_eq!(report.failures().count(), 1);
}

#[test]
fn an_empty_report_does_not_claim_a_pass() {
    assert!(!HardGateReport::default().passed());
}

#[test]
fn only_the_complete_fixed_order_can_pass() {
    let outcomes = HardGate::ORDER
        .into_iter()
        .map(|gate| HardGateOutcome::pass(gate, GateContext::Trial))
        .collect();
    let report = HardGateReport::new(outcomes);

    assert!(report.ordered_prefix_is_valid());
    assert!(report.passed());
}

#[test]
fn an_omission_or_reorder_cannot_claim_a_pass() {
    let omitted = HardGateReport::new(vec![HardGateOutcome::pass(
        HardGate::CrashOrUnexpectedContact,
        GateContext::Trial,
    )]);
    let reordered = HardGateReport::new(vec![
        HardGateOutcome::pass(HardGate::TrialIdentity, GateContext::Trial),
        HardGateOutcome::pass(HardGate::CrashOrUnexpectedContact, GateContext::Trial),
    ]);

    assert!(omitted.ordered_prefix_is_valid());
    assert!(!omitted.passed());
    assert!(!reordered.ordered_prefix_is_valid());
    assert!(!reordered.passed());
}
