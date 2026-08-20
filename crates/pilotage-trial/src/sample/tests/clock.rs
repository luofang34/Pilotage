use super::*;

#[test]
fn same_epoch_clock_regression_rejects_a_discontinuity_record() {
    let run = run_identity();
    let previous = sample(&run);
    let mut current = sample(&run);
    current.sequence = 2;
    current.time.simulator = clock_reading(29);
    current
        .time
        .clock_discontinuities
        .push(ClockDomain::Simulator);

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(
            ValidationError::ClockRegression { .. }
        ))
    ));
}

#[test]
fn clock_epoch_change_requires_a_discontinuity_and_mapping() {
    let mut run = run_identity();
    let mut next_epoch = mapping(ClockDomain::Simulator);
    next_epoch.source_epoch = 8;
    next_epoch.source_anchor_ns = 30;
    next_epoch.recorder_anchor_ns = 40;
    run.clock_mappings.push(next_epoch);
    let previous = sample(&run);
    let mut current = sample(&run);
    current.sequence = 2;
    current.time.simulator = Observed::present(ClockReading {
        epoch: 8,
        time_ns: 30,
    });
    current.simulator_truth.stamp.source.epoch = 8;

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(
            ValidationError::InvalidClockObservation { .. }
        ))
    ));
    current
        .time
        .clock_discontinuities
        .push(ClockDomain::Simulator);
    assert!(current.validate_after(&previous, &run).is_ok());
}

#[test]
fn recorder_clock_rejects_a_discontinuity_marker() {
    let run = run_identity();
    let mut sample = sample(&run);
    sample
        .time
        .clock_discontinuities
        .push(ClockDomain::Recorder);

    assert!(matches!(
        sample.validate_local(),
        Err(ValidationError::InvalidClockObservation { .. })
    ));
}

#[test]
fn public_clock_accessor_preserves_the_epoch() {
    let run = run_identity();
    let value = sample(&run);

    assert_eq!(
        value.time.reading(ClockDomain::Simulator),
        Some(ClockReading {
            epoch: 7,
            time_ns: 30,
        })
    );
    assert_eq!(
        value.time.reading(ClockDomain::Recorder),
        Some(ClockReading {
            epoch: 0,
            time_ns: 40,
        })
    );
}

#[test]
fn causal_stage_rejects_receive_apply_reversal() {
    let run = run_identity();
    let mut sample = sample(&run);
    sample.raw_input.stamp.recorder_apply_ns = 19;

    assert!(matches!(
        sample.validate_local(),
        Err(ValidationError::InvalidStageStamp { field, .. }) if field == "sample.raw_input"
    ));
}

#[test]
fn causal_stage_rejects_an_unexpected_producer_role() {
    let run = run_identity();
    let mut sample = sample(&run);
    sample.simulator_truth.stamp.source.producer = StageProducerRole::ControlClient;

    assert!(matches!(
        sample.validate_local(),
        Err(ValidationError::InvalidStageStamp { field, .. })
            if field == "sample.simulator_truth"
    ));
}

#[test]
fn causal_stage_rejects_a_mapped_source_time_after_receive() {
    let run = run_identity();
    let mut sample = sample(&run);
    sample.raw_input.stamp.source.time_ns = Observed::present(12);

    assert!(matches!(
        sample.validate_for_run(&run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn causal_stage_rejects_a_source_sequence_regression() {
    let run = run_identity();
    let mut previous = sample(&run);
    previous.raw_input.stamp.source.sequence = 5;
    let mut current = sample(&run);
    current.sequence = 2;
    current.raw_input.stamp.source.sequence = 4;

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn one_source_sequence_cannot_change_its_observation() {
    let run = run_identity();
    let previous = sample(&run);
    let mut current = sample(&run);
    current.sequence = 2;
    current.raw_input.observation = Observed::present(RawInput {
        axes: vec![0.9],
        buttons: vec![false],
    });

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn one_source_sequence_cannot_change_its_event_times() {
    let run = run_identity();
    let previous = sample(&run);
    let mut current = sample(&run);
    current.sequence = 2;
    current.raw_input.stamp.recorder_receive_ns = 22;
    current.raw_input.stamp.recorder_apply_ns = 31;

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn causal_stage_source_clock_cannot_change_inside_one_run() {
    let run = run_identity();
    let previous = sample(&run);
    let mut current = sample(&run);
    current.sequence = 2;
    current.raw_input.stamp.source.clock = ClockDomain::Simulator;
    current.raw_input.stamp.source.time_ns = Observed::present(10);

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn causal_stage_discontinuity_must_change_the_epoch() {
    let run = run_identity();
    let mut previous = sample(&run);
    previous.time.device = Observed::missing(crate::MissingReason::NotPublished, None);
    let mut current = sample(&run);
    current.sequence = 2;
    current.time.device = Observed::missing(crate::MissingReason::NotPublished, None);
    current.time.clock_discontinuities.push(ClockDomain::Device);

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn causal_stage_epoch_change_requires_a_discontinuity() {
    let mut run = run_identity();
    let mut next_epoch = mapping(ClockDomain::Device);
    next_epoch.source_epoch = 8;
    run.clock_mappings.push(next_epoch);
    let mut previous = sample(&run);
    previous.time.device = Observed::missing(crate::MissingReason::NotPublished, None);
    let mut current = sample(&run);
    current.sequence = 2;
    current.time.device = Observed::missing(crate::MissingReason::NotPublished, None);
    current.raw_input.stamp.source.epoch = 8;

    assert!(matches!(
        current.validate_after(&previous, &run),
        Err(CodecError::Validation(ValidationError::InvalidStageStamp {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
    current.time.clock_discontinuities.push(ClockDomain::Device);
    assert!(current.validate_after(&previous, &run).is_ok());
}

#[test]
fn present_stage_can_record_a_missing_source_time_without_a_fake_mapping() {
    let mut run = run_identity();
    run.clock_mappings
        .retain(|mapping| mapping.from != ClockDomain::Device);
    let mut sample = sample(&run);
    sample.time.device = Observed::missing(crate::MissingReason::NotPublished, None);
    sample.raw_input.stamp.source.time_ns = Observed::missing(
        crate::MissingReason::NotPublished,
        Some("device time was not published".to_owned()),
    );

    assert!(sample.validate_for_run(&run).is_ok());
}

#[test]
fn present_sample_clock_requires_its_epoch_mapping() {
    let mut run = run_identity();
    run.clock_mappings
        .retain(|mapping| mapping.from != ClockDomain::Simulator);
    let sample = sample(&run);

    assert!(matches!(
        sample.validate_for_run(&run),
        Err(CodecError::Validation(ValidationError::MissingClockMapping {
            field,
            ..
        })) if field == "sample.time.simulator"
    ));
}

#[test]
fn sample_clock_must_map_to_the_recorder_sample_time() {
    let run = run_identity();
    let mut sample = sample(&run);
    sample.time.simulator = clock_reading(28);

    assert!(matches!(
        sample.validate_for_run(&run),
        Err(CodecError::Validation(
            ValidationError::InvalidClockObservation { field, .. }
        )) if field == "sample.time.simulator"
    ));
}

#[test]
fn missing_stage_can_record_a_missing_source_time() {
    let run = run_identity();
    let mut sample = sample(&run);
    let mut stamp = sample.raw_input.stamp.clone();
    stamp.source.time_ns = Observed::missing(crate::MissingReason::NotPublished, None);
    sample.raw_input = CausalStage::missing(
        stamp,
        crate::MissingReason::NotPublished,
        Some("the device did not publish a report".to_owned()),
    );

    assert!(sample.validate_for_run(&run).is_ok());
}

#[test]
fn present_stage_requires_its_exact_clock_epoch() {
    let run = run_identity();
    let mut sample = sample(&run);
    sample.raw_input.stamp.source.epoch = 8;

    assert!(matches!(
        sample.validate_for_run(&run),
        Err(CodecError::Validation(ValidationError::MissingClockMapping {
            field,
            epoch: 8,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn present_stage_rejects_time_outside_the_mapping_interval() {
    let run = run_identity();
    let mut sample = sample(&run);
    sample.raw_input.stamp.source.time_ns = Observed::present(1_001);

    assert!(matches!(
        sample.validate_for_run(&run),
        Err(CodecError::Validation(ValidationError::ClockTimeOutsideMapping {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}

#[test]
fn present_stage_rejects_an_unusable_mapping() {
    let mut run = run_identity();
    let device_mapping = run
        .clock_mappings
        .iter_mut()
        .find(|mapping| mapping.from == ClockDomain::Device)
        .expect("device mapping");
    device_mapping.quality = ClockMappingQuality::Unusable;
    let mut sample = sample(&run);
    sample.time.device = Observed::missing(crate::MissingReason::NotPublished, None);

    assert!(matches!(
        sample.validate_for_run(&run),
        Err(CodecError::Validation(ValidationError::UnusableClockMapping {
            field,
            ..
        })) if field == "sample.raw_input"
    ));
}
