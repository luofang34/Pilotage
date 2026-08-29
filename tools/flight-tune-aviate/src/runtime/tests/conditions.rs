//! The condition set one scored window was flown under.

use crate::runtime::conditions::ConditionLedger;

use super::frame;

fn with_conditions(sequence: u64, values: &[(&str, f64)]) -> flight_tune::ScenarioFrame {
    let mut value = frame(sequence, sequence * 1_000_000);
    value.applied_conditions = values
        .iter()
        .map(|(name, number)| ((*name).to_owned(), *number))
        .collect();
    value
}

#[test]
fn a_ledger_records_the_conditions_a_run_observes() {
    let mut ledger = ConditionLedger::new();
    ledger
        .observe(&with_conditions(1, &[("wind.speed_mps", 3.0)]))
        .expect("record the applied conditions");
    assert_eq!(ledger.observed().get("wind.speed_mps"), Some(&3.0));
    assert!(!ledger.is_locked());
}

#[test]
fn a_condition_cannot_change_inside_a_scored_window() {
    let mut ledger = ConditionLedger::new();
    ledger
        .observe(&with_conditions(1, &[("wind.speed_mps", 3.0)]))
        .expect("record the applied conditions");
    ledger.lock();

    ledger
        .observe(&with_conditions(2, &[("wind.speed_mps", 3.0)]))
        .expect("the same value keeps the window valid");
    let detail = ledger
        .observe(&with_conditions(3, &[("wind.speed_mps", 9.0)]))
        .expect_err("a changed value must end the window")
        .to_string();
    assert!(detail.contains("wind.speed_mps"), "{detail}");
}

#[test]
fn a_new_condition_cannot_appear_inside_a_scored_window() {
    let mut ledger = ConditionLedger::new();
    ledger
        .observe(&with_conditions(1, &[("wind.speed_mps", 3.0)]))
        .expect("record the applied conditions");
    ledger.lock();
    ledger
        .observe(&with_conditions(
            2,
            &[("wind.speed_mps", 3.0), ("wind.gust_mps", 1.0)],
        ))
        .expect_err("a new condition must end the window");
}

#[test]
fn a_condition_free_window_freezes_the_empty_set() {
    let mut ledger = ConditionLedger::new();
    ledger.lock();
    assert!(ledger.is_locked());
    assert!(ledger.observed().is_empty());
    // A condition that appears inside the window still ends it.
    ledger
        .observe(&with_conditions(1, &[("wind.speed_mps", 3.0)]))
        .expect_err("a condition appearing inside a window must end it");
    ledger.unlock();
    ledger
        .observe(&with_conditions(1, &[("wind.speed_mps", 3.0)]))
        .expect("record the applied conditions");
    ledger.lock();
    ledger.unlock();
    ledger
        .observe(&with_conditions(2, &[("wind.speed_mps", 9.0)]))
        .expect("an unlocked ledger accepts a new condition set");
    ledger.clear();
    assert!(ledger.observed().is_empty());
}
