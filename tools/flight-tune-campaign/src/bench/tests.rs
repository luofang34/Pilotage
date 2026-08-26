#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use flight_tune::{BoundedCoordinateSearch, CandidateLineage, FinalQualificationOutcome, Tuner};
use pilotage_control_feel::FeelMode;

use super::*;
use crate::FlightQualityEvaluator;

/// A directory that is removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        // The resolved temporary root, not the symlink to it: the durable
        // store refuses to follow a symlinked path component, which is what
        // stops a campaign writing through a link somebody else controls.
        #[cfg(target_os = "macos")]
        let root = PathBuf::from("/private/tmp");
        #[cfg(not(target_os = "macos"))]
        let root = std::env::temp_dir();
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = root.join(format!(
            "pilotage-bench-{name}-{}-{time}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&path).ok();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn start_candidate() -> Candidate {
    Candidate::new(
        CandidateLineage {
            schema: "pilotage.bench.candidate.v1".to_owned(),
            base_preset_digest: Digest::from_bytes([2; 32]),
            plant_digest: Digest::from_bytes([3; 32]),
        },
        warm_start_parameters(FeelMode::Balanced),
    )
    .expect("build the warm start")
}

/// Runs one complete campaign for one vehicle and returns its final verdict.
fn campaign(
    name: &str,
    model: BenchVehicle,
    promotion: flight_tune::PromotionPolicy,
    qualification: flight_tune::QualificationPolicy,
) -> (FinalQualificationOutcome, PathBuf) {
    let scratch = Scratch::new(name);
    let root = scratch.0.clone();
    let handle = BenchHandle::default();
    let mut tuner = Tuner::open_or_resume(
        &root,
        bench_stage(name, promotion, qualification),
        4_242,
        start_candidate(),
        BenchBackend::new(
            handle.clone(),
            model,
            &format!("{name}-airframe"),
            Digest::from_bytes([4; 32]),
        )
        .expect("build backend"),
        BenchVehicleFactory::new(handle, name).expect("build vehicle factory"),
        BenchGates::new(model.full_scale_mps * 1.5, 60.0).expect("build gates"),
        FlightQualityEvaluator::new(Digest::from_bytes([9; 32])).expect("build evaluator"),
        BoundedCoordinateSearch::new(0.25).expect("build search"),
    )
    .expect("open the campaign");

    tuner
        .run_training_attempts_blocking(3)
        .expect("run training attempts");
    tuner.freeze_candidate().expect("freeze the champion");
    tuner
        .run_promotion_once_blocking()
        .expect("run the promotion decision");
    let outcome = tuner
        .run_final_qualification_once_blocking()
        .expect("run final qualification");

    let published = root.join("published");
    crate::publish_journal_evidence_blocking(tuner.journal(), &published)
        .expect("publish the campaign evidence");
    // The scratch directory outlives this call only through the returned path,
    // which the caller reads before dropping it.
    std::mem::forget(scratch);
    (outcome, root)
}

#[test]
fn a_campaign_runs_end_to_end_for_the_alia250() {
    // The whole chain: a warm start, a bounded search over the command law,
    // a frozen champion, a paired promotion decision on scenarios the search
    // never saw, a final bar on scenarios neither saw, and published evidence
    // an independent verifier reads back.
    let (outcome, root) = campaign(
        "alia250",
        BenchVehicle::alia250(),
        crate::alia250_promotion_policy(),
        crate::alia250_qualification_policy(),
    );
    assert!(
        matches!(
            outcome,
            FinalQualificationOutcome::Qualified
                | FinalQualificationOutcome::FailedObjective { .. }
        ),
        "the campaign reached a verdict rather than an error: {outcome:?}"
    );
    assert!(root.join("published").exists(), "evidence was published");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_campaign_runs_end_to_end_for_the_x500() {
    // The second vehicle contributes a model and a bar and nothing else. That
    // this runs at all, through the same backend, evaluator and engine, is the
    // reusability claim made concrete.
    let (outcome, root) = campaign(
        "x500",
        BenchVehicle::x500(),
        crate::x500_promotion_policy(),
        crate::x500_qualification_policy(),
    );
    assert!(
        matches!(
            outcome,
            FinalQualificationOutcome::Qualified
                | FinalQualificationOutcome::FailedObjective { .. }
        ),
        "the campaign reached a verdict rather than an error: {outcome:?}"
    );
    assert!(root.join("published").exists(), "evidence was published");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_search_cannot_propose_a_law_that_releases_slower_than_it_applies() {
    // The search moves a release FACTOR rather than a release rate, floored at
    // one. A search free to set the release rate directly could propose a law
    // that took longer to stop commanding than to start, which is the one
    // ordering a control law must not have.
    let mut parameters = warm_start_parameters(FeelMode::Balanced);
    parameters.insert(parameter::RELEASE_FACTOR.to_owned(), 0.1);
    let proposed = Candidate::new(
        CandidateLineage {
            schema: "pilotage.bench.candidate.v1".to_owned(),
            base_preset_digest: Digest::from_bytes([2; 32]),
            plant_digest: Digest::from_bytes([3; 32]),
        },
        parameters,
    )
    .expect("build candidate");
    let response = response_from(&proposed).expect("read response");
    assert!(response.dynamics.release_accel >= response.dynamics.apply_accel);
    assert!(response.dynamics.release_jerk >= response.dynamics.apply_jerk);
}

#[test]
fn a_search_cannot_propose_a_band_with_no_hysteresis() {
    // Leaving must be harder than staying, or an input resting on the edge
    // chatters between commanding and not.
    let response = response_from(&start_candidate()).expect("read response");
    assert!(response.neutral.active_exit < response.neutral.active_enter);
}

#[test]
fn the_warm_start_is_the_law_the_vehicle_would_otherwise_ship() {
    // A search that started from nothing would spend its first trials
    // rediscovering that a control has to be stable.
    let parameters = warm_start_parameters(FeelMode::Balanced);
    let shipped = pilotage_control_feel::FlightFeelProfile::shaped(FeelMode::Balanced).horizontal;
    assert!(
        (parameters[parameter::APPLY_ACCEL] - f64::from(shipped.dynamics.apply_accel)).abs() < 1e-9
    );
    assert!(parameters[parameter::NEUTRAL_DWELL_MS] > 0.0);
    // And it is inside the bounds the search may move within, or the first
    // proposal would be a correction rather than a step.
    let stage = bench_stage(
        "bounds",
        crate::alia250_promotion_policy(),
        crate::alia250_qualification_policy(),
    );
    for (name, bounds) in &stage.allowlist {
        let value = parameters[name];
        assert!(
            bounds.minimum <= value && value <= bounds.maximum,
            "{name} starts at {value}, outside {}..{}",
            bounds.minimum,
            bounds.maximum
        );
    }
}
