//! Partition tests: instance 0 keeps the default partition, every other
//! session gets one no other backend or instance shares.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use super::{partition, partition_claims, partition_env};
use crate::backend::SessionContext;
use crate::conflicts::Claim;

fn context(instance: u16) -> SessionContext {
    let mut ctx = SessionContext::for_test(PathBuf::from("unused"));
    ctx.instance = instance;
    ctx
}

#[test]
fn instance_zero_keeps_the_default_partition_and_claims_it() {
    let ctx = context(0);
    assert_eq!(partition(&ctx, "px4-gz"), None);
    assert!(partition_env(&ctx, "px4-gz").is_empty());
    assert_eq!(
        partition_claims(&ctx, "px4-gz"),
        vec![Claim::DefaultGzPartition]
    );
}

#[test]
fn every_other_session_gets_a_partition_of_its_own() {
    let five = partition(&context(5), "px4-gz").expect("instance 5 is partitioned");
    let six = partition(&context(6), "px4-gz").expect("instance 6 is partitioned");
    let other_backend = partition(&context(5), "aviate-gz").expect("partitioned");
    assert_ne!(five, six, "instances never share a partition");
    assert_ne!(five, other_backend, "backends never share a partition");
    assert_eq!(
        partition_env(&context(5), "px4-gz"),
        vec![("GZ_PARTITION".to_owned(), five)]
    );
    assert!(partition_claims(&context(5), "px4-gz").is_empty());
}
