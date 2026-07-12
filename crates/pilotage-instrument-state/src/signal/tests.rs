#![allow(clippy::expect_used, clippy::panic)]

use super::{FreshnessPolicy, SignalStatus};

#[test]
fn severity_order_is_valid_to_failed() {
    assert!(SignalStatus::Valid < SignalStatus::Degraded);
    assert!(SignalStatus::Degraded < SignalStatus::Stale);
    assert!(SignalStatus::Stale < SignalStatus::Missing);
    assert!(SignalStatus::Missing < SignalStatus::Failed);
}

#[test]
fn worst_picks_the_more_severe() {
    assert_eq!(
        SignalStatus::Valid.worst(SignalStatus::Stale),
        SignalStatus::Stale
    );
    assert_eq!(
        SignalStatus::Failed.worst(SignalStatus::Valid),
        SignalStatus::Failed
    );
}

#[test]
fn age_resolves_through_the_thresholds() {
    let p = FreshnessPolicy::default();
    assert_eq!(p.status_for_age(Some(0.0)), SignalStatus::Valid);
    assert_eq!(p.status_for_age(Some(749.0)), SignalStatus::Valid);
    assert_eq!(p.status_for_age(Some(750.0)), SignalStatus::Stale);
    assert_eq!(p.status_for_age(Some(3000.0)), SignalStatus::Failed);
    assert_eq!(p.status_for_age(None), SignalStatus::Missing);
    assert_eq!(p.status_for_age(Some(f32::NAN)), SignalStatus::Missing);
    assert_eq!(p.status_for_age(Some(-1.0)), SignalStatus::Missing);
}

#[test]
fn only_showable_statuses_show_values() {
    assert!(SignalStatus::Valid.shows_value());
    assert!(SignalStatus::Degraded.shows_value());
    assert!(SignalStatus::Stale.shows_value());
    assert!(!SignalStatus::Missing.shows_value());
    assert!(!SignalStatus::Failed.shows_value());
}

// ---- VAL-01 policy construction -----------------------------------------------

#[test]
fn invalid_policy_construction_is_rejected() {
    use super::PolicyError;
    for (stale, fail) in [
        (f32::NAN, 100.0),
        (100.0, f32::NAN),
        (f32::INFINITY, 200.0),
        (100.0, f32::INFINITY),
        (0.0, 100.0),
        (-1.0, 100.0),
        (100.0, 0.0),
    ] {
        assert_eq!(
            super::FreshnessPolicy::new(stale, fail),
            Err(PolicyError::NonPositiveThreshold),
            "{stale}/{fail}"
        );
    }
    assert_eq!(
        super::FreshnessPolicy::new(100.0, 100.0),
        Err(PolicyError::StaleNotBeforeFail)
    );
    assert_eq!(
        super::FreshnessPolicy::new(200.0, 100.0),
        Err(PolicyError::StaleNotBeforeFail)
    );
    let policy = super::FreshnessPolicy::new(100.0, 200.0).expect("valid thresholds");
    assert_eq!(policy.stale_after_ms(), 100.0);
    assert_eq!(policy.fail_after_ms(), 200.0);
}
