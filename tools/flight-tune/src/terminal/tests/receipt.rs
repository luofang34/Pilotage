use serde_json::Value;

use super::{
    SemanticCase, binding, fixed_digest, successful_outcomes, terminal_intent, terminal_receipt,
    terminal_report,
};
use crate::terminal::{
    RunTerminalClass, RunTerminalDisposition, RunTerminalQuarantine, RunTerminalReceipt,
};

#[test]
fn both_completed_receipts_validate() {
    for case in [SemanticCase::ScenarioComplete, SemanticCase::HardGateAbort] {
        let receipt = terminal_receipt(case);
        assert!(receipt.is_completed());
        receipt.validate().expect("validate completed receipt");
    }
}

#[test]
fn evidence_failure_creates_one_quarantine_receipt() {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let report = terminal_report(&intent, successful_outcomes());
    let class = RunTerminalClass::evidence_failure(&intent, &report)
        .expect("create evidence failure class");
    let receipt =
        RunTerminalReceipt::new(&binding(&intent), &intent, &report, class, fixed_digest(93))
            .expect("create evidence failure receipt");
    assert!(!receipt.is_completed());
    assert_eq!(
        receipt.class().disposition(),
        RunTerminalDisposition::Quarantine {
            quarantine: RunTerminalQuarantine::EvidenceFailure
        }
    );
    receipt.validate().expect("validate quarantine receipt");
}

#[test]
fn completed_class_in_a_quarantine_variant_fails() {
    let receipt = terminal_receipt(SemanticCase::ScenarioComplete);
    let mut document = serde_json::to_value(receipt).expect("encode receipt");
    document["receipt"] = Value::from("quarantine");
    let changed: RunTerminalReceipt =
        serde_json::from_value(document).expect("decode changed receipt");
    assert!(changed.validate().is_err());
}

#[test]
fn quarantine_class_in_a_completed_variant_fails() {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let report = terminal_report(&intent, successful_outcomes());
    let class = RunTerminalClass::evidence_failure(&intent, &report)
        .expect("create evidence failure class");
    let receipt =
        RunTerminalReceipt::new(&binding(&intent), &intent, &report, class, fixed_digest(93))
            .expect("create quarantine receipt");
    let mut document = serde_json::to_value(receipt).expect("encode receipt");
    document["receipt"] = Value::from("completed");
    let changed: RunTerminalReceipt =
        serde_json::from_value(document).expect("decode changed receipt");
    assert!(changed.validate().is_err());
}

#[test]
fn changed_class_or_receipt_digest_fails() {
    for field in ["class", "receipt_digest"] {
        let receipt = terminal_receipt(SemanticCase::ScenarioComplete);
        let mut document = serde_json::to_value(receipt).expect("encode receipt");
        match field {
            "class" => {
                document["class"]["disposition"]["completion"] = Value::from("hard_gate_abort");
            }
            "receipt_digest" => {
                document[field] =
                    serde_json::to_value(fixed_digest(71)).expect("encode changed digest");
            }
            _ => panic!("unknown test field"),
        }
        let changed: RunTerminalReceipt =
            serde_json::from_value(document).expect("decode changed receipt");
        assert!(changed.validate().is_err(), "field {field}");
    }
}

#[test]
fn changed_causal_evidence_fails_canonical_validation() {
    let receipt = terminal_receipt(SemanticCase::ScenarioComplete);
    let mut document = serde_json::to_value(receipt).expect("encode receipt");
    document["causal_evidence_digest"] =
        serde_json::to_value(fixed_digest(72)).expect("encode causal digest");
    let changed: RunTerminalReceipt =
        serde_json::from_value(document).expect("decode changed receipt");
    assert!(changed.validate().is_err());
}
