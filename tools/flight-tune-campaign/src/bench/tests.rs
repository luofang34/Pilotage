#![allow(clippy::expect_used, clippy::panic)]

mod open_rollback;

use std::path::PathBuf;

use flight_tune::{
    BoundedCoordinateSearch, Candidate, CandidateLineage, FinalQualificationOutcome, Tuner,
};
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

/// Scores one candidate on the bench trial, through the shipped shaper and
/// the shipped evaluator.
///
/// The trace is the one the backend produces, so a result here is a result the
/// campaign would have measured.
fn score_candidate(model: BenchVehicle, candidate: &Candidate) -> flight_tune::MetricValues {
    use flight_tune::MetricEvaluator as _;
    let response = super::response_from(candidate).expect("response");
    let mut shaper = pilotage_control_feel::AxisDemandShaper::default();
    let mut evaluator =
        FlightQualityEvaluator::new(Digest::from_bytes([9; 32])).expect("evaluator");
    evaluator
        .begin(&bench_scenario("training-step", model).expect("build scenario"))
        .expect("begin");
    let (mut velocity, mut position, mut previous) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut step: u32 = 0;
    loop {
        let time_s = f64::from(step) * super::DT_S;
        if time_s >= super::END_S {
            break;
        }
        let (phase, stick) = super::BenchBackend::input_at(time_s);
        let shaped = shaper.step(stick, 1.0, super::DT_S as f32, response).value;
        let demanded = f64::from(shaped) * model.full_scale_mps;
        velocity += (demanded - velocity) * super::DT_S / model.time_constant_s;
        position += velocity * super::DT_S;
        let acceleration = (velocity - previous) / super::DT_S;
        previous = velocity;
        let sample = flight_tune::TelemetrySample {
            sequence: u64::from(step),
            elapsed_ms: u64::from(step) * 20,
            values: std::collections::BTreeMap::from([
                (channel::COMMAND.to_owned(), f64::from(shaped)),
                (
                    channel::RESPONSE.to_owned(),
                    velocity / model.full_scale_mps,
                ),
                (channel::POSITION_M.to_owned(), position),
                (channel::VELOCITY_MPS.to_owned(), velocity),
                (channel::ACCELERATION_MPS2.to_owned(), acceleration),
                (channel::PHYSICAL_DEMAND.to_owned(), demanded),
                (channel::EFFORT.to_owned(), f64::from(shaped.abs())),
                (
                    channel::SATURATED.to_owned(),
                    f64::from(u8::from(shaped.abs() >= 0.999)),
                ),
                (channel::CRASHED.to_owned(), 0.0),
                (channel::GROUND_CONTACT.to_owned(), 0.0),
                (channel::PHASE.to_owned(), phase),
            ]),
        };
        evaluator.observe(&sample).expect("observe");
        step = step.wrapping_add(1);
    }
    evaluator.finish().expect("finish")
}

/// Runs one complete campaign for one vehicle and returns its final verdict.
fn campaign(
    name: &str,
    model: BenchVehicle,
    promotion: flight_tune::PromotionPolicy,
    qualification: flight_tune::QualificationPolicy,
    response_targets: flight_tune::ResponseTargetTable,
) -> (FinalQualificationOutcome, PathBuf) {
    campaign_with_attempts(name, model, promotion, qualification, response_targets, 3)
}

/// The same chain with a stated training budget, so the smoke can fly
/// the whole thing at a fraction of the certification's journal cost.
fn campaign_with_attempts(
    name: &str,
    model: BenchVehicle,
    promotion: flight_tune::PromotionPolicy,
    qualification: flight_tune::QualificationPolicy,
    response_targets: flight_tune::ResponseTargetTable,
    training_attempts: u64,
) -> (FinalQualificationOutcome, PathBuf) {
    let scratch = Scratch::new(name);
    let root = scratch.0.clone();
    let handle = BenchHandle::default();
    let mut tuner = Tuner::open_or_resume(
        &root,
        bench_stage(name, model, promotion, qualification, response_targets).expect("build stage"),
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
        .run_training_attempts_blocking(training_attempts)
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

/// The declared training budget for one bench campaign.
///
/// The run count is exact, so a stage whose suites grow past what the budget
/// pays for fails here and not halfway through a run. The duration is the
/// ceiling every run may take at the bench's declared sample timeout, not the
/// time a campaign takes: an observed campaign finishes far under it.
const BENCH_MAXIMUM_RUNS: u64 = 34;
const BENCH_MAXIMUM_DURATION_NS: u64 = 9_520_000_000_000;

/// A prepared campaign states a finite run and duration bound before it runs.
///
/// The bound counts every challenger run and every suite baseline a challenger
/// can require, so an operator can read the cost of the search from the stage
/// rather than from the wall clock.
#[test]
fn each_vehicle_states_a_finite_bound_inside_its_declared_budget() {
    for (name, model, promotion, qualification, response_targets) in [
        (
            "alia250",
            BenchVehicle::alia250(),
            crate::alia250_promotion_policy(),
            crate::alia250_qualification_policy(),
            crate::alia250_response_targets().expect("build response targets"),
        ),
        (
            "x500",
            BenchVehicle::x500(),
            crate::x500_promotion_policy(),
            crate::x500_qualification_policy(),
            crate::x500_response_targets().expect("build response targets"),
        ),
    ] {
        let stage = bench_stage(name, model, promotion, qualification, response_targets)
            .expect("build stage");
        let bound = stage.run_bound(3).expect("state a bounded campaign");

        assert_eq!(
            bound.maximum_runs, BENCH_MAXIMUM_RUNS,
            "the {name} campaign states its declared run bound"
        );
        assert!(
            bound.maximum_duration_ns <= BENCH_MAXIMUM_DURATION_NS,
            "the {name} campaign fits its declared duration budget: {}",
            bound.maximum_duration_ns
        );
    }
}

/// The full chain at smoke cost: one vehicle, one training attempt,
/// every phase, and the winner still qualifies. This is the per-merge
/// answer to "did I break the engine or the bench wiring"; the
/// certification pair below answers the larger question on its own
/// cadence.
#[test]
fn a_campaign_smokes_the_whole_chain() {
    let (outcome, root) = campaign_with_attempts(
        "smoke",
        BenchVehicle::alia250(),
        crate::alia250_promotion_policy(),
        crate::alia250_qualification_policy(),
        crate::alia250_response_targets().expect("build response targets"),
        1,
    );
    assert!(
        matches!(outcome, FinalQualificationOutcome::Qualified),
        "the smoke campaign qualifies its winner: {outcome:?}"
    );
    assert!(root.join("published").exists(), "evidence was published");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
#[ignore = "certification: the full campaign for one vehicle, run by its own CI step"]
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
        crate::alia250_response_targets().expect("build response targets"),
    );
    // Qualified, not merely "a verdict": a bar no legal candidate can
    // reach would seal FailedObjective on every run, and this assertion
    // is what distinguishes a working bar from a decorative one.
    assert!(
        matches!(outcome, FinalQualificationOutcome::Qualified),
        "the campaign qualifies its winner: {outcome:?}"
    );
    assert!(root.join("published").exists(), "evidence was published");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
#[ignore = "certification: the full campaign for one vehicle, run by its own CI step"]
fn a_campaign_runs_end_to_end_for_the_x500() {
    // The second vehicle contributes a model and a bar and nothing else. That
    // this runs at all, through the same backend, evaluator and engine, is the
    // reusability claim made concrete.
    let (outcome, root) = campaign(
        "x500",
        BenchVehicle::x500(),
        crate::x500_promotion_policy(),
        crate::x500_qualification_policy(),
        crate::x500_response_targets().expect("build response targets"),
    );
    assert!(
        matches!(outcome, FinalQualificationOutcome::Qualified),
        "the campaign qualifies its winner: {outcome:?}"
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
        BenchVehicle::alia250(),
        crate::alia250_promotion_policy(),
        crate::alia250_qualification_policy(),
        crate::alia250_response_targets().expect("build response targets"),
    )
    .expect("build stage");
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

/// Prints the shipped warm start's measured objectives on this bench's
/// trial, for recalibrating the vehicle bars when the trial or the
/// models change. Run with `--ignored --nocapture`.
#[test]
#[ignore = "calibration probe, prints with --nocapture"]
#[allow(clippy::disallowed_macros)] // an ignored diagnostic exists to print
fn probe_warm_start_objectives() {
    for (name, model) in [
        ("alia250", BenchVehicle::alia250()),
        ("x500", BenchVehicle::x500()),
    ] {
        let values = score_candidate(model, &start_candidate());

        eprintln!("=== {name} loss={} objectives:", values.loss);
        for (key, value) in &values.objectives {
            eprintln!("  {key} = {value:.4}");
        }
    }
}

/// A candidate that gives away operator authority is refused by the band, and
/// by nothing else.
///
/// This is the failure a normalized response metric cannot see. A larger expo
/// resolves less physical speed for the same stick, so the vehicle reaches its
/// smaller target sooner and overshoots it less, and every normalized
/// measurement reads that as a better command law. The three halves are
/// asserted together on purpose: the metrics improve, the physical target
/// drops, and the band is the only thing that refuses it.
#[test]
fn a_candidate_that_lowers_its_physical_target_fails_the_authority_band() {
    let model = BenchVehicle::alia250();
    let shipped = start_candidate();
    let mut parameters = warm_start_parameters(FeelMode::Balanced);
    parameters.insert(parameter::CENTER_EXPO.to_owned(), 0.7);
    parameters.insert(parameter::OUTER_EXPO.to_owned(), 0.7);
    parameters.insert(parameter::DEADZONE.to_owned(), 0.12);
    let quiet = Candidate::new(
        CandidateLineage {
            schema: "pilotage.bench.candidate.v1".to_owned(),
            base_preset_digest: Digest::from_bytes([2; 32]),
            plant_digest: Digest::from_bytes([3; 32]),
        },
        parameters,
    )
    .expect("build the quieter candidate");

    let shipped_values = score_candidate(model, &shipped);
    let quiet_values = score_candidate(model, &quiet);
    let objective = |values: &flight_tune::MetricValues, name: &str| {
        *values
            .objectives
            .get(name)
            .unwrap_or_else(|| panic!("{name} is measured"))
    };

    // One: the normalized response metrics do not get worse.
    let shipped_settling = objective(&shipped_values, "response.settling_time_s");
    let quiet_settling = objective(&quiet_values, "response.settling_time_s");
    assert!(
        quiet_settling <= shipped_settling,
        "the quieter candidate settles no later: {quiet_settling} against {shipped_settling}"
    );
    let shipped_overshoot = objective(&shipped_values, "response.overshoot_fraction");
    let quiet_overshoot = objective(&quiet_values, "response.overshoot_fraction");
    assert!(
        quiet_overshoot <= shipped_overshoot,
        "the quieter candidate overshoots no more: {quiet_overshoot} against {shipped_overshoot}"
    );

    // Two: it resolved a smaller physical target for the same operator input.
    let name = flight_tune::TARGET_AUTHORITY_OBJECTIVE;
    let shipped_target = objective(&shipped_values, name);
    let quiet_target = objective(&quiet_values, name);
    assert!(
        quiet_target < shipped_target,
        "the quieter candidate asks for less speed: {quiet_target} against {shipped_target}"
    );

    // Three: the band is what refuses it, and the shipped law clears it.
    let table = crate::alia250_response_targets().expect("build response targets");
    let promotion = crate::bench_mission_revision_id(crate::BENCH_PROMOTION_TRIAL_ID, model)
        .expect("promotion mission identity");
    assert!(
        table.authority_holds(&promotion, Some(shipped_target)),
        "the shipped law keeps its authority at {shipped_target} m/s"
    );
    assert!(
        !table.authority_holds(&promotion, Some(quiet_target)),
        "the quieter candidate gave away authority at {quiet_target} m/s"
    );
}
