use super::*;

fn stream<'run>(run: &'run RunIdentity) -> TrialStreamValidator<'run> {
    TrialStreamValidator::new(run).expect("validated stream")
}

fn next_sample(run: &RunIdentity, sequence: u64) -> TrialSample {
    let mut value = sample(run);
    value.sequence = sequence;
    value
}

fn missing<T>(stage: &mut CausalStage<T>) {
    stage.stamp.predecessor = None;
    stage.observation = Observed::missing(crate::MissingReason::NotPublished, None);
}

fn make_control_tail_missing(value: &mut TrialSample) {
    missing(&mut value.normalized_control);
    missing(&mut value.typed_intent);
    missing(&mut value.adapter_demand);
    missing(&mut value.transmitted_setpoint);
}

fn add_epoch(run: &mut RunIdentity, domain: ClockDomain, epoch: u64) {
    let mut value = mapping(domain);
    value.source_epoch = epoch;
    run.clock_mappings.push(value);
}

#[test]
fn complete_stream_accepts_one_correlated_sample() {
    let run = run_identity();
    let value = sample(&run);
    let mut validator = stream(&run);

    assert!(validator.validate_next(&value).is_ok());
    assert_eq!(validator.validated_samples(), 1);
}

#[test]
fn single_sample_rejects_the_wrong_predecessor_stage() {
    let run = run_identity();
    let mut value = sample(&run);
    value
        .normalized_control
        .stamp
        .predecessor
        .as_mut()
        .expect("predecessor")
        .stage = ControlStage::TypedIntent;

    assert!(matches!(
        value.validate_for_run(&run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.normalized_control"
    ));
}

#[test]
fn stream_rejects_an_unknown_predecessor_event() {
    let run = run_identity();
    let source = event(ControlStage::RawInput, ClockDomain::Device, 1);
    let unknown = [
        ControlEventId {
            clock: ClockDomain::Simulator,
            ..source
        },
        ControlEventId { epoch: 8, ..source },
        ControlEventId {
            sequence: 99,
            ..source
        },
    ];

    for predecessor in unknown {
        let mut value = sample(&run);
        value.normalized_control.stamp.predecessor = Some(predecessor);
        let mut validator = stream(&run);
        assert!(matches!(
            validator.validate_next(&value),
            Err(CodecError::Validation(
                ValidationError::UnknownControlPredecessor { field, .. }
            )) if field == "sample.normalized_control"
        ));
        assert_eq!(validator.validated_samples(), 0);
    }
}

#[test]
fn present_derived_event_requires_a_predecessor() {
    let run = run_identity();
    let mut value = sample(&run);
    value.normalized_control.stamp.predecessor = None;

    assert!(matches!(
        value.validate_for_run(&run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.normalized_control"
    ));
}

#[test]
fn nonderived_stages_cannot_claim_a_control_predecessor() {
    let run = run_identity();
    let predecessor = Some(event(ControlStage::RawInput, ClockDomain::Device, 1));
    for field in [
        "sample.raw_input",
        "sample.flight_controller_estimate",
        "sample.simulator_truth",
    ] {
        let mut value = sample(&run);
        match field {
            "sample.raw_input" => value.raw_input.stamp.predecessor = predecessor,
            "sample.flight_controller_estimate" => {
                value.flight_controller_estimate.stamp.predecessor = predecessor;
            }
            _ => value.simulator_truth.stamp.predecessor = predecessor,
        }
        assert!(matches!(
            value.validate_for_run(&run),
            Err(CodecError::Validation(ValidationError::InvalidStageStamp {
                field: actual,
                ..
            })) if actual == field
        ));
    }
}

#[test]
fn pipeline_lag_can_reference_a_prior_raw_event() {
    let run = run_identity();
    let first = sample(&run);
    let mut second = next_sample(&run, 2);
    second.raw_input.stamp.source.sequence = 2;
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    assert!(validator.validate_next(&second).is_ok());
}

#[test]
fn explicit_missing_control_stages_are_valid_stream_records() {
    let run = run_identity();
    let mut value = sample(&run);
    make_control_tail_missing(&mut value);
    let mut validator = stream(&run);

    assert!(validator.validate_next(&value).is_ok());
}

#[test]
fn a_present_stage_cannot_reference_a_missing_upstream_event() {
    let run = run_identity();
    let mut value = sample(&run);
    missing(&mut value.raw_input);
    let mut validator = stream(&run);

    assert!(matches!(
        validator.validate_next(&value),
        Err(CodecError::Validation(ValidationError::UnknownControlPredecessor {
            field,
            ..
        })) if field == "sample.normalized_control"
    ));
}

#[test]
fn recorder_observation_order_does_not_replace_source_lineage() {
    let run = run_identity();
    let mut value = sample(&run);
    value.raw_input.stamp.recorder_apply_ns = 24;
    let mut validator = stream(&run);

    assert!(validator.validate_next(&value).is_ok());
}

#[test]
fn mapped_event_time_cannot_be_definitely_before_the_predecessor() {
    let run = run_identity();
    let mut value = sample(&run);
    value.raw_input.stamp.source.time_ns = Observed::present(15);
    value.raw_input.stamp.recorder_receive_ns = 26;
    value.raw_input.stamp.recorder_apply_ns = 26;
    value.normalized_control.stamp.recorder_receive_ns = 30;
    value.normalized_control.stamp.recorder_apply_ns = 30;
    let mut validator = stream(&run);

    assert!(matches!(
        validator.validate_next(&value),
        Err(CodecError::Validation(ValidationError::CausalMappedSourceOrder {
            field,
            ..
        })) if field == "sample.normalized_control"
    ));
}

#[test]
fn overlapping_mapped_intervals_remain_valid_bounded_evidence() {
    let run = run_identity();
    let value = sample(&run);
    let mut validator = stream(&run);

    assert!(validator.validate_next(&value).is_ok());
}

#[test]
fn stage_time_regression_after_a_missing_record_is_rejected() {
    let run = run_identity();
    let first = sample(&run);
    let mut missing_time = next_sample(&run, 2);
    missing_time.raw_input.stamp.source.sequence = 2;
    missing_time.raw_input.stamp.source.time_ns =
        Observed::missing(crate::MissingReason::NotPublished, None);
    let mut regression = next_sample(&run, 3);
    regression.raw_input.stamp.source.sequence = 3;
    regression.raw_input.stamp.source.time_ns = Observed::present(5);
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    validator
        .validate_next(&missing_time)
        .expect("missing source time");
    assert!(matches!(
        validator.validate_next(&regression),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn stage_time_can_resume_forward_after_a_missing_record() {
    let run = run_identity();
    let first = sample(&run);
    let mut missing_time = next_sample(&run, 2);
    missing_time.raw_input.stamp.source.sequence = 2;
    missing_time.raw_input.stamp.source.time_ns =
        Observed::missing(crate::MissingReason::NotPublished, None);
    let mut resumed = next_sample(&run, 3);
    resumed.raw_input.stamp.source.sequence = 3;
    resumed.raw_input.stamp.source.time_ns = Observed::present(11);
    resumed.raw_input.stamp.recorder_receive_ns = 22;
    resumed.raw_input.stamp.recorder_apply_ns = 22;
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    validator
        .validate_next(&missing_time)
        .expect("missing source time");
    assert!(validator.validate_next(&resumed).is_ok());
}

#[test]
fn sample_clock_regression_after_a_missing_reading_is_rejected() {
    let run = run_identity();
    let first = sample(&run);
    let mut missing_time = next_sample(&run, 2);
    missing_time.time.device = Observed::missing(crate::MissingReason::NotPublished, None);
    let mut regression = next_sample(&run, 3);
    regression.time.device = clock_reading(29);
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    validator
        .validate_next(&missing_time)
        .expect("missing clock reading");
    assert!(matches!(
        validator.validate_next(&regression),
        Err(CodecError::Validation(
            ValidationError::ClockRegression { .. }
        ))
    ));
}

#[test]
fn pending_discontinuity_accepts_a_new_epoch_after_missing() {
    let mut run = run_identity();
    add_epoch(&mut run, ClockDomain::Simulator, 8);
    let first = sample(&run);
    let mut discontinuity = next_sample(&run, 2);
    discontinuity.time.simulator =
        Observed::missing(crate::MissingReason::ClockDiscontinuity, None);
    discontinuity
        .time
        .clock_discontinuities
        .push(ClockDomain::Simulator);
    discontinuity.simulator_truth.stamp.source.epoch = 8;
    let mut resumed = discontinuity.clone();
    resumed.sequence = 3;
    resumed.time.simulator = Observed::present(ClockReading {
        epoch: 8,
        time_ns: 30,
    });
    resumed.time.clock_discontinuities.clear();
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    validator
        .validate_next(&discontinuity)
        .expect("discontinuity sample");
    assert!(validator.validate_next(&resumed).is_ok());
}

#[test]
fn pending_discontinuity_rejects_a_return_to_the_same_epoch() {
    let mut run = run_identity();
    add_epoch(&mut run, ClockDomain::Simulator, 8);
    let first = sample(&run);
    let mut discontinuity = next_sample(&run, 2);
    discontinuity.time.simulator =
        Observed::missing(crate::MissingReason::ClockDiscontinuity, None);
    discontinuity
        .time
        .clock_discontinuities
        .push(ClockDomain::Simulator);
    discontinuity.simulator_truth.stamp.source.epoch = 8;
    let mut resumed = discontinuity.clone();
    resumed.sequence = 3;
    resumed.time.simulator = clock_reading(30);
    resumed.time.clock_discontinuities.clear();
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    validator
        .validate_next(&discontinuity)
        .expect("discontinuity sample");
    assert!(matches!(
        validator.validate_next(&resumed),
        Err(CodecError::Validation(
            ValidationError::InvalidClockObservation { .. }
        ))
    ));
}

#[test]
fn failed_validation_does_not_change_stream_state() {
    let run = run_identity();
    let first = sample(&run);
    let mut invalid = next_sample(&run, 2);
    invalid.time.device = clock_reading(29);
    let mut corrected = next_sample(&run, 2);
    corrected.time.device = clock_reading(31);
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    assert!(validator.validate_next(&invalid).is_err());
    assert_eq!(validator.validated_samples(), 1);
    assert!(validator.validate_next(&corrected).is_ok());
    assert_eq!(validator.validated_samples(), 2);
}

#[test]
fn declared_sample_loss_does_not_authorize_clock_regression() {
    let run = run_identity();
    let first = sample(&run);
    let mut current = next_sample(&run, 3);
    current.dropped_before = 1;
    current.time.device = clock_reading(29);
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    assert!(matches!(
        validator.validate_next(&current),
        Err(CodecError::Validation(
            ValidationError::ClockRegression { .. }
        ))
    ));
}

#[test]
fn declared_sample_loss_does_not_authorize_an_unknown_predecessor() {
    let run = run_identity();
    let first = sample(&run);
    let mut current = next_sample(&run, 3);
    current.dropped_before = 1;
    current.raw_input.stamp.source.sequence = 3;
    current.normalized_control.stamp.source.sequence = 3;
    current.normalized_control.stamp.predecessor =
        Some(event(ControlStage::RawInput, ClockDomain::Device, 2));
    let mut validator = stream(&run);

    validator.validate_next(&first).expect("first sample");
    assert!(matches!(
        validator.validate_next(&current),
        Err(CodecError::Validation(
            ValidationError::UnknownControlPredecessor { .. }
        ))
    ));
}

#[test]
fn unconsumed_control_event_history_has_a_hard_limit() {
    let run = run_identity();
    let first = sample(&run);
    let mut validator = stream(&run);
    validator.validate_next(&first).expect("first sample");

    for sequence in 2..=u64::try_from(crate::MAX_CONTROL_EVENT_HISTORY).expect("history limit") {
        let mut value = next_sample(&run, sequence);
        value.raw_input.stamp.source.sequence = sequence;
        validator.validate_next(&value).expect("bounded backlog");
    }
    let sequence = u64::try_from(crate::MAX_CONTROL_EVENT_HISTORY)
        .expect("history limit")
        .wrapping_add(1);
    let mut overflow = next_sample(&run, sequence);
    overflow.raw_input.stamp.source.sequence = sequence;

    assert!(matches!(
        validator.validate_next(&overflow),
        Err(CodecError::Validation(ValidationError::TooManyItems {
            limit: crate::MAX_CONTROL_EVENT_HISTORY,
            ..
        }))
    ));
}

#[test]
fn missing_downstream_cannot_grow_event_history_without_a_limit() {
    let run = run_identity();
    let mut first = sample(&run);
    make_control_tail_missing(&mut first);
    let mut validator = stream(&run);
    validator.validate_next(&first).expect("first sample");

    for sequence in 2..=u64::try_from(crate::MAX_CONTROL_EVENT_HISTORY).expect("history limit") {
        let mut value = next_sample(&run, sequence);
        value.raw_input.stamp.source.sequence = sequence;
        make_control_tail_missing(&mut value);
        validator.validate_next(&value).expect("bounded backlog");
    }
    let sequence = u64::try_from(crate::MAX_CONTROL_EVENT_HISTORY)
        .expect("history limit")
        .wrapping_add(1);
    let mut overflow = next_sample(&run, sequence);
    overflow.raw_input.stamp.source.sequence = sequence;
    make_control_tail_missing(&mut overflow);

    assert!(matches!(
        validator.validate_next(&overflow),
        Err(CodecError::Validation(ValidationError::TooManyItems {
            limit: crate::MAX_CONTROL_EVENT_HISTORY,
            ..
        }))
    ));
}
