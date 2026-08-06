//! Ingress admission, source resets, and the fail-closed authorization
//! regimes, transliterated from the behaviors the browser suite pins.

#![allow(clippy::expect_used, clippy::panic)]

use super::{
    AttitudeGroup, AvionicsIngress, AvionicsSample, Coherence, IncarnationPolicy, IngressConfig,
    KinematicsGroup,
};
use crate::stamp::{CLOCK_SIMULATION, ROLE_OPERATIONAL_ESTIMATE, RawStamp};

const VEHICLE: u64 = 42;
const SKEW_BUDGET_NS: u64 = 5_000_000;

fn config(policy: IncarnationPolicy) -> IngressConfig {
    IngressConfig {
        vehicle_id: VEHICLE,
        source_id: None,
        incarnation: None,
        incarnation_policy: policy,
        maximum_seen_incarnations: 8,
        maximum_skew_nanos: SKEW_BUDGET_NS,
    }
}

fn stamp(incarnation: u8, epoch: u32, sequence: u32, at_ns: u64) -> RawStamp {
    RawStamp {
        role: ROLE_OPERATIONAL_ESTIMATE,
        integrity: 2,
        source_id: 7,
        incarnation: [incarnation; 16],
        epoch,
        sequence,
        acquired_at_ns: at_ns,
        clock: CLOCK_SIMULATION,
    }
}

fn sample(sequence: u32, at_ns: u64) -> AvionicsSample {
    let stamp = stamp(1, 1, sequence, at_ns);
    AvionicsSample {
        vehicle_id: VEHICLE,
        attitude: AttitudeGroup {
            quat: [1.0, 0.0, 0.0, 0.0],
            rates: [0.0; 3],
            arm_state: 0,
        },
        kinematics: KinematicsGroup {
            pos_ned: [0.0; 3],
            vel_ned: [0.0; 3],
            arm_state: 0,
        },
        valid_flags: 0b1111,
        quality: 0,
        attitude_stamp: Some(stamp),
        kinematics_stamp: Some(stamp),
        estimator_status_stamp: Some(stamp),
    }
}

#[test]
fn a_full_publication_is_admitted_and_authorized() {
    let mut ingress = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    assert!(ingress.ingest(&sample(1, 1_000_000), 0.0));
    let snapshot = ingress.snapshot(10.0);
    assert_eq!(snapshot.valid_flags, 0b1111);
    assert_eq!(snapshot.quality, 0);
    assert_eq!(snapshot.coherence.status, Coherence::Coherent);
    assert!(snapshot.attitude.is_some());
    assert_eq!(snapshot.attitude.expect("admitted").age_ms, 10.0);
}

#[test]
fn duplicates_reorders_and_time_regressions_never_refresh() {
    let mut ingress = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    assert!(ingress.ingest(&sample(5, 5_000_000), 0.0));
    // Duplicate sequence.
    assert!(!ingress.ingest(&sample(5, 6_000_000), 1.0));
    // Serially older sequence.
    assert!(!ingress.ingest(&sample(4, 7_000_000), 2.0));
    // Newer sequence but regressed acquisition time.
    assert!(!ingress.ingest(&sample(6, 4_000_000), 3.0));
    let (counters, _) = ingress.diagnostics();
    assert_eq!(counters.duplicates, 3);
    assert_eq!(counters.reordered, 3);
    assert_eq!(counters.time_regressions, 3);
    // The admitted group's age still runs from the original acceptance.
    let snapshot = ingress.snapshot(10.0);
    assert_eq!(snapshot.attitude.expect("admitted").age_ms, 10.0);
}

#[test]
fn sequence_gaps_are_counted_by_distance() {
    let mut ingress = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    assert!(ingress.ingest(&sample(1, 1_000_000), 0.0));
    assert!(ingress.ingest(&sample(5, 2_000_000), 1.0));
    let (counters, _) = ingress.diagnostics();
    // Three groups each skipped sequences 2..=4.
    assert_eq!(counters.sequence_gaps, 9);
}

#[test]
fn wrong_vehicle_and_wrong_source_are_refused() {
    let mut ingress = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    let mut foreign = sample(1, 1_000_000);
    foreign.vehicle_id = VEHICLE + 1;
    assert!(!ingress.ingest(&foreign, 0.0));
    assert!(ingress.ingest(&sample(1, 1_000_000), 0.0));
    let mut other_source = sample(2, 2_000_000);
    let mut stamp = other_source.attitude_stamp.expect("stamp");
    stamp.source_id = 8;
    other_source.attitude_stamp = Some(stamp);
    other_source.kinematics_stamp = None;
    other_source.estimator_status_stamp = None;
    assert!(!ingress.ingest(&other_source, 1.0));
    let (counters, _) = ingress.diagnostics();
    assert_eq!(counters.wrong_vehicle, 1);
    assert_eq!(counters.wrong_source, 1);
}

#[test]
fn an_epoch_advance_is_a_source_reset_that_clears_groups() {
    let mut ingress = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    assert!(ingress.ingest(&sample(9, 9_000_000), 0.0));
    let mut reset = sample(1, 500_000);
    for slot in [
        &mut reset.attitude_stamp,
        &mut reset.kinematics_stamp,
        &mut reset.estimator_status_stamp,
    ] {
        let mut stamp = slot.expect("stamp");
        stamp.epoch = 2;
        *slot = Some(stamp);
    }
    assert!(ingress.ingest(&reset, 1.0));
    let (counters, _) = ingress.diagnostics();
    // The status group hits the epoch first and clears; the two numeric
    // groups then admit under the already-advanced epoch.
    assert_eq!(counters.source_resets, 1);
    let snapshot = ingress.snapshot(2.0);
    assert_eq!(snapshot.epoch, Some(2));
    // The old epoch is a replay afterward.
    assert!(!ingress.ingest(&sample(10, 10_000_000), 2.0));
    let (counters, _) = ingress.diagnostics();
    assert_eq!(counters.old_epoch, 3);
}

#[test]
fn pin_first_refuses_unseen_incarnations_and_sim_policy_resets() {
    let mut pinned = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    assert!(pinned.ingest(&sample(1, 1_000_000), 0.0));
    let mut moved = sample(2, 2_000_000);
    for slot in [
        &mut moved.attitude_stamp,
        &mut moved.kinematics_stamp,
        &mut moved.estimator_status_stamp,
    ] {
        let mut stamp = slot.expect("stamp");
        stamp.incarnation = [9; 16];
        *slot = Some(stamp);
    }
    assert!(!pinned.ingest(&moved, 1.0));
    let (counters, _) = pinned.diagnostics();
    assert_eq!(counters.wrong_incarnation, 3);

    let mut sim = AvionicsIngress::new(config(IncarnationPolicy::SimAcceptUnseen));
    assert!(sim.ingest(&sample(1, 1_000_000), 0.0));
    assert!(sim.ingest(&moved, 1.0));
    let (counters, _) = sim.diagnostics();
    assert_eq!(counters.incarnation_transitions, 1);
    // Returning to the ALREADY-SEEN first incarnation is a replay.
    assert!(!sim.ingest(&sample(3, 3_000_000), 2.0));
    let (counters, _) = sim.diagnostics();
    assert_eq!(counters.old_incarnation, 3);
}

#[test]
fn a_duplicate_status_downgrade_is_monotone_and_sticks() {
    let mut ingress = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    assert!(ingress.ingest(&sample(1, 1_000_000), 0.0));
    assert_eq!(ingress.snapshot(1.0).valid_flags, 0b1111);

    // The SAME status stamp republished with tightened trust: flags
    // fold by AND, quality by max — and only fail-closed direction.
    let mut downgrade = sample(1, 1_000_000);
    downgrade.attitude_stamp = None;
    downgrade.kinematics_stamp = None;
    downgrade.valid_flags = 0b0011;
    downgrade.quality = 1;
    assert!(ingress.ingest(&downgrade, 1.0));
    let snapshot = ingress.snapshot(2.0);
    assert_eq!(snapshot.valid_flags, 0b0011);
    assert_eq!(snapshot.quality, 1);

    // An attempted upgrade over the same stamp restores nothing.
    let mut upgrade = downgrade;
    upgrade.valid_flags = 0b1111;
    upgrade.quality = 0;
    ingress.ingest(&upgrade, 2.0);
    let snapshot = ingress.snapshot(3.0);
    assert_eq!(snapshot.valid_flags, 0b0011);
    assert_eq!(snapshot.quality, 1);

    // A LATER numeric acquired before the downgrade cannot resurrect
    // the pre-downgrade authorization: the current regime caps it.
    let mut late_numeric = sample(2, 1_500_000);
    late_numeric.estimator_status_stamp = Some(stamp(1, 1, 1, 1_000_000));
    late_numeric.valid_flags = 0b1111;
    late_numeric.quality = 0;
    ingress.ingest(&late_numeric, 3.0);
    let snapshot = ingress.snapshot(4.0);
    assert_eq!(
        snapshot.valid_flags & 0b1100,
        0b0000,
        "kinematics stay capped"
    );
    assert_eq!(
        snapshot.valid_flags & 0b0011,
        0b0011,
        "attitude bits pass the fold"
    );
}

#[test]
fn numerics_without_a_pairable_status_fail_closed() {
    let mut ingress = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    let mut orphan = sample(1, 1_000_000);
    orphan.estimator_status_stamp = None;
    assert!(ingress.ingest(&orphan, 0.0));
    let snapshot = ingress.snapshot(1.0);
    assert_eq!(snapshot.valid_flags, 0);
    assert_eq!(snapshot.quality, 2);

    // A status acquired far beyond the skew budget cannot vouch either.
    let mut skewed = sample(2, 100_000_000);
    skewed.attitude_stamp = Some(stamp(1, 1, 2, 100_000_000));
    skewed.kinematics_stamp = None;
    skewed.estimator_status_stamp = Some(stamp(1, 1, 2, 100_000_000 + SKEW_BUDGET_NS + 1));
    ingress.ingest(&skewed, 1.0);
    let snapshot = ingress.snapshot(2.0);
    assert_eq!(snapshot.valid_flags & 0b0011, 0, "attitude unpaired");
}

#[test]
fn excessive_skew_between_groups_is_a_counted_transition() {
    let mut ingress = AvionicsIngress::new(config(IncarnationPolicy::PinFirst));
    let mut split = sample(1, 1_000_000);
    let far = stamp(1, 1, 1, 1_000_000 + SKEW_BUDGET_NS + 1);
    split.kinematics_stamp = Some(far);
    assert!(ingress.ingest(&split, 0.0));
    let snapshot = ingress.snapshot(1.0);
    assert_eq!(snapshot.coherence.status, Coherence::ExcessiveSkew);
    let (counters, _) = ingress.diagnostics();
    assert_eq!(counters.excessive_skew, 1);
    // A second publication that KEEPS the skew excessive is not a new
    // transition: the counter pins the edge, not the condition.
    let mut still_split = sample(2, 2_000_000);
    still_split.kinematics_stamp = Some(stamp(1, 1, 2, 2_000_000 + SKEW_BUDGET_NS + 1));
    assert!(ingress.ingest(&still_split, 1.0));
    let (counters, _) = ingress.diagnostics();
    assert_eq!(counters.excessive_skew, 1);
}
