#![allow(clippy::expect_used, clippy::panic)]

use super::{
    EXECUTION_RETRY_POLICY_SCHEMA_VERSION, ExecutionRetryPolicy, MAX_EXECUTION_RETRY_LIMIT,
};

#[test]
fn the_no_replacement_policy_permits_nothing() {
    let policy = ExecutionRetryPolicy::none();

    assert_eq!(policy.execution_retry_limit, 0);
    assert!(!policy.permits_replacement(0));
}

#[test]
fn a_limit_permits_exactly_that_many_replacements() {
    let policy = ExecutionRetryPolicy::with_limit(2).expect("a supported limit");

    assert!(policy.permits_replacement(0));
    assert!(policy.permits_replacement(1));
    assert!(!policy.permits_replacement(2));
}

#[test]
fn a_limit_above_the_maximum_is_rejected() {
    let over = MAX_EXECUTION_RETRY_LIMIT.wrapping_add(1);

    assert!(ExecutionRetryPolicy::with_limit(over).is_err());
}

#[test]
fn an_unsupported_schema_is_rejected() {
    let policy = ExecutionRetryPolicy {
        schema_version: EXECUTION_RETRY_POLICY_SCHEMA_VERSION.wrapping_add(1),
        execution_retry_limit: 0,
    };

    assert!(policy.validate().is_err());
}

#[test]
fn a_changed_limit_changes_the_canonical_bytes() {
    let none = serde_json::to_vec(&ExecutionRetryPolicy::none()).expect("encode no-retry");
    let one = serde_json::to_vec(&ExecutionRetryPolicy::with_limit(1).expect("limit"))
        .expect("encode one-retry");

    assert_ne!(none, one);
}
