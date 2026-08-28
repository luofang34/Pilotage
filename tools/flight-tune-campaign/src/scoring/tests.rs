#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use flight_tune::{Digest, MetricEvaluator, MissionReference, TelemetrySample};

use super::{FlightQualityEvaluator, channel};

/// One trial: a step in, a hold, a release, and a settle.
///
/// The plant is a first-order lag, which is enough to produce every family of
/// measurement without pretending to be an aircraft. What is under test is the
/// bridge from samples to named objectives, not a vehicle.
fn trial() -> Vec<TelemetrySample> {
    let mut samples = Vec::new();
    let mut position = 0.0_f64;
    let mut velocity = 0.0_f64;
    let mut previous_velocity = 0.0_f64;
    let dt = 0.02_f64;
    for step in 0..600_u32 {
        let time_s = f64::from(step) * dt;
        let (phase, command) = match time_s {
            t if t < 0.5 => (0.0, 0.0),
            t if t < 3.0 => (1.0, 1.0),
            t if t < 5.0 => (2.0, 1.0),
            t if t < 7.0 => (3.0, 0.0),
            _ => (4.0, 0.0),
        };
        velocity += (command - velocity) * dt / 0.25;
        position += velocity * dt;
        let acceleration = (velocity - previous_velocity) / dt;
        previous_velocity = velocity;
        samples.push(TelemetrySample {
            sequence: u64::from(step),
            elapsed_ms: u64::from(step) * 20,
            values: BTreeMap::from([
                (channel::COMMAND.to_owned(), command),
                (channel::RESPONSE.to_owned(), velocity),
                (channel::POSITION_M.to_owned(), position),
                (channel::VELOCITY_MPS.to_owned(), velocity),
                (channel::ACCELERATION_MPS2.to_owned(), acceleration),
                (channel::EFFORT.to_owned(), command.abs()),
                (channel::SATURATED.to_owned(), 0.0),
                (channel::PHASE.to_owned(), phase),
            ]),
        });
    }
    samples
}

fn scenario() -> MissionReference {
    MissionReference {
        revision_id: "step-hold-release".to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: Digest::from_bytes([7; 32]),
        max_samples: 1_000,
        sample_timeout_ns: 200_000_000,
    }
}

fn score() -> flight_tune::MetricValues {
    let mut evaluator =
        FlightQualityEvaluator::new(Digest::from_bytes([9; 32])).expect("build evaluator");
    evaluator.begin(&scenario()).expect("begin run");
    for sample in trial() {
        evaluator.observe(&sample).expect("observe sample");
    }
    evaluator.finish().expect("finish run")
}

#[test]
fn the_evaluator_produces_exactly_what_both_vehicles_are_scored_on() {
    // This join decides whether a campaign can score anything. Final
    // qualification requires every objective a bar names to be present in
    // every run, so a bar naming one the evaluator does not produce fails the
    // whole campaign on the name — after every run has been flown.
    let produced: BTreeSet<String> = score().objectives.keys().cloned().collect();
    for (vehicle, qualification) in [
        ("alia250", crate::alia250_qualification_policy()),
        ("x500", crate::x500_qualification_policy()),
    ] {
        let required: BTreeSet<String> = qualification.objective_maxima.keys().cloned().collect();
        assert_eq!(
            required, produced,
            "{vehicle} is scored on objectives the evaluator does not produce"
        );
    }
}

#[test]
fn every_objective_it_produces_is_one_the_scoring_layer_admits() {
    for name in score().objectives.keys() {
        assert!(
            pilotage_flight_quality::is_producible(name),
            "{name} is produced but not in the scoring vocabulary"
        );
    }
}

#[test]
fn the_scored_values_are_finite_and_nonnegative() {
    // A bar compares each objective against a maximum. A value that is not a
    // number compares false against every limit, which reads as passing.
    let values = score();
    assert!(
        values.loss.is_finite() && values.loss >= 0.0,
        "loss {}",
        values.loss
    );
    assert!(
        values.control_effort.is_finite() && (0.0..=1.0).contains(&values.control_effort),
        "control effort {}",
        values.control_effort
    );
    for (name, value) in &values.objectives {
        assert!(value.is_finite() && *value >= 0.0, "{name} is {value}");
    }
}

#[test]
fn a_run_that_never_released_is_refused_rather_than_measured() {
    // Release metrics are defined from the moment of release. A run that never
    // released has none, and inventing them would score a brake that never
    // happened.
    let mut evaluator =
        FlightQualityEvaluator::new(Digest::from_bytes([9; 32])).expect("build evaluator");
    evaluator.begin(&scenario()).expect("begin run");
    for sample in trial()
        .into_iter()
        .filter(|sample| sample.values[channel::PHASE] < 3.0)
    {
        evaluator.observe(&sample).expect("observe sample");
    }
    let refused = evaluator.finish().expect_err("a run with no release");
    assert!(
        refused.to_string().contains("never released"),
        "unexpected refusal: {refused}"
    );
}

#[test]
fn a_sample_missing_a_channel_is_refused_by_name() {
    // A backend that stopped reporting one channel would otherwise score a run
    // on whatever the missing channel defaulted to.
    let mut evaluator =
        FlightQualityEvaluator::new(Digest::from_bytes([9; 32])).expect("build evaluator");
    evaluator.begin(&scenario()).expect("begin run");
    let mut sample = trial().remove(0);
    sample.values.remove(channel::VELOCITY_MPS);
    let refused = evaluator
        .observe(&sample)
        .expect_err("a sample with no velocity");
    assert!(
        refused.to_string().contains(channel::VELOCITY_MPS),
        "the refusal must name what was missing: {refused}"
    );
}

#[test]
fn a_sample_outside_a_run_is_refused() {
    let mut evaluator =
        FlightQualityEvaluator::new(Digest::from_bytes([9; 32])).expect("build evaluator");
    let sample = trial().remove(0);
    assert!(evaluator.observe(&sample).is_err());
    // And a cancelled run leaves nothing behind for the next one.
    evaluator.begin(&scenario()).expect("begin run");
    evaluator.observe(&sample).expect("observe sample");
    evaluator.cancel().expect("cancel");
    assert!(evaluator.observe(&sample).is_err());
}
