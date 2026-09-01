//! What a stream accepts, and every way one sample can fail to answer.

use super::super::*;
use super::support::{GOLDEN_RUN_SEED, stream_condition};
use crate::Digest;

const BASELINE_FORCE_BITS: u32 = 0x3f00_0000;
const HOLD_INTERVAL: u32 = 10;
const LANE_COUNT: u8 = 4;

#[test]
fn a_complete_stream_counts_every_factor_it_verified() {
    let declaration = declaration();
    let samples = run(&declaration, 21);
    let summary = accept_all(&declaration, &samples).expect("verified stream");

    assert_eq!(summary.ledger.sample_count, 21);
    assert_eq!(summary.ledger.actuator.commanded, 21);
    assert_eq!(summary.ledger.actuator.eligible, 21);
    assert_eq!(summary.ledger.actuator.primed, 1);
    assert_eq!(summary.ledger.actuator.selected_hold, 2);
    assert_eq!(summary.ledger.actuator.applied_hold, 2);
    assert_eq!(summary.ledger.actuator.scaled, 21);
    assert_eq!(summary.ledger.actuator.bypassed.total(), 0);

    let lane = summary.ledger.sensor_lanes[0];
    assert_eq!(lane.lane_tag, 0);
    assert_eq!(lane.eligible, 21);
    // One offset is drawn every two samples and held for the second.
    assert_eq!(lane.held, 10);
    assert!(lane.changed > 0);
}

#[test]
fn a_receipt_binds_the_counts_to_the_samples_that_made_them() {
    let declaration = declaration();
    let samples = run(&declaration, 21);
    let summary = accept_all(&declaration, &samples).expect("verified stream");
    let receipt = receipt(&declaration, &summary);

    receipt.validate().expect("sealed receipt");

    let mut changed = receipt.clone();
    changed.ledger.actuator.applied_hold = 1;
    assert!(changed.validate().is_err());
}

#[test]
fn a_repeated_sample_sequence_is_refused() {
    assert_refused(|samples| samples[5] = samples[4]);
}

#[test]
fn a_sample_sequence_gap_is_refused() {
    assert_refused(|samples| {
        samples[5].sequence = samples[5].sequence.wrapping_add(1);
    });
}

#[test]
fn a_sample_sequence_rewind_is_refused() {
    assert_refused(|samples| samples[5].global_sample_sequence = 0);
}

#[test]
fn a_changed_effective_sensor_value_is_refused() {
    assert_refused(|samples| {
        let mut sensor = samples[5].sensor.expect("sensor evidence");
        sensor.effective_value_bits[0] = Some(0x3f80_0000);
        sensor.effective_digest =
            derivation::sensor_sample_digest(sensor.presence_mask, &sensor.effective_value_bits);
        samples[5].sensor = Some(sensor);
    });
}

#[test]
fn a_sensor_identity_that_does_not_cover_its_values_is_refused() {
    assert_refused(|samples| {
        let mut sensor = samples[5].sensor.expect("sensor evidence");
        sensor.effective_digest = Digest::from_bytes([9; 32]);
        samples[5].sensor = Some(sensor);
    });
}

#[test]
fn a_changed_undeclared_sensor_lane_is_refused() {
    assert_refused(|samples| {
        let mut sensor = samples[5].sensor.expect("sensor evidence");
        sensor.effective_value_bits[3] = Some(0x4048_0000);
        sensor.changed_mask |= 1 << 3;
        sensor.effective_digest =
            derivation::sensor_sample_digest(sensor.presence_mask, &sensor.effective_value_bits);
        samples[5].sensor = Some(sensor);
    });
}

#[test]
fn a_hold_decision_the_schedule_never_stated_is_refused() {
    assert_refused(|samples| {
        let mut actuator = samples[5].actuator.expect("actuator evidence");
        actuator.selected_hold = !actuator.selected_hold;
        actuator.applied_hold = actuator.selected_hold;
        samples[5].actuator = Some(actuator);
    });
}

#[test]
fn an_interval_identity_that_was_not_derived_is_refused() {
    assert_refused(|samples| {
        for sample in samples.iter_mut() {
            let Some(mut actuator) = sample.actuator else {
                continue;
            };
            if actuator.interval_identity.is_some() {
                actuator.interval_identity = Some(Digest::from_bytes([4; 32]));
                sample.actuator = Some(actuator);
                return;
            }
        }
    });
}

#[test]
fn a_bypassed_command_that_was_scaled_is_refused() {
    assert_refused(|samples| {
        let mut actuator = samples[5].actuator.expect("actuator evidence");
        actuator.eligibility = ExecutedEligibility::Bypass(ExecutedBypassReason::Failsafe);
        actuator.selected_hold = false;
        actuator.applied_hold = false;
        actuator.interval_epoch = None;
        actuator.interval_index = None;
        actuator.interval_position = None;
        actuator.interval_identity = None;
        samples[5].actuator = Some(actuator);
    });
}

#[test]
fn a_safety_command_that_reaches_the_actuator_whole_is_accepted() {
    let declaration = declaration();
    let mut samples = run(&declaration, 21);
    let mut actuator = samples[5].actuator.expect("actuator evidence");
    actuator.eligibility = ExecutedEligibility::Bypass(ExecutedBypassReason::Failsafe);
    actuator.authority_scaled_lane_bits = actuator.requested_lane_bits;
    actuator.effective_lane_bits = actuator.requested_lane_bits;
    actuator.selected_hold = false;
    actuator.applied_hold = false;
    actuator.interval_epoch = None;
    actuator.interval_index = None;
    actuator.interval_position = None;
    actuator.interval_identity = None;
    actuator.interval_complete = false;
    samples[5].actuator = Some(actuator);

    // Every later sample belongs to the epoch the bypass reset, so the
    // stream stops at the first sample that carries the old one.
    let mut stream = ExecutedStream::open(&declaration).expect("stream");
    for sample in samples.iter().take(6) {
        stream.accept(sample).expect("accepted through the bypass");
    }
    assert_eq!(stream.ledger().actuator.bypassed.failsafe, 1);
    assert_eq!(stream.ledger().actuator.eligible, 5);
}

#[test]
fn an_active_online_hover_estimator_is_refused() {
    assert_refused(|samples| samples[5].hover.estimator_disabled = false);
}

#[test]
fn a_hover_force_that_moves_inside_one_run_is_refused() {
    assert_refused(|samples| {
        samples[5].hover.baseline_force_bits = 0x3f40_0000;
        samples[5].hover.effective_force_bits = derivation::scaled_hover_force(
            samples[5].hover.baseline_force_bits,
            samples[5].hover.scale_basis_points,
        );
    });
}

#[test]
fn a_hover_scale_the_declaration_never_stated_is_refused() {
    assert_refused(|samples| {
        samples[5].hover.scale_basis_points = 10_000;
        samples[5].hover.effective_force_bits =
            derivation::scaled_hover_force(samples[5].hover.baseline_force_bits, 10_000);
    });
}

#[test]
fn a_sample_with_no_completed_lockstep_answer_is_refused() {
    assert_refused(|samples| samples[5].send.lockstep = false);
}

#[test]
fn a_stream_that_ends_inside_an_interval_is_refused() {
    let declaration = declaration();
    let samples = run(&declaration, 15);

    assert!(accept_all(&declaration, &samples).is_err());
}

fn assert_refused(change: impl FnOnce(&mut Vec<ExecutedSample>)) {
    let declaration = declaration();
    let samples = run(&declaration, 21);
    assert!(
        accept_all(&declaration, &samples).is_ok(),
        "the unchanged stream must verify"
    );

    let mut changed = samples;
    change(&mut changed);
    assert!(
        accept_all(&declaration, &changed).is_err(),
        "the changed stream must be refused"
    );
}

fn accept_all(
    declaration: &ExecutedUncertaintyDeclaration,
    samples: &[ExecutedSample],
) -> Result<ExecutedStreamSummary, crate::TuneError> {
    let mut stream = ExecutedStream::open(declaration)?;
    for sample in samples {
        stream.accept(sample)?;
    }
    stream.close()
}

fn receipt(
    declaration: &ExecutedUncertaintyDeclaration,
    summary: &ExecutedStreamSummary,
) -> ExecutedUncertaintyReceipt {
    let launch = ExecutedLaunchIdentity::new(
        Digest::from_bytes([5; 32]),
        declaration.artifact_digest,
        declaration.condition_digest,
        declaration.run_seed,
        declaration.required_capabilities.clone(),
        3,
    )
    .expect("launch identity");
    ExecutedUncertaintyReceipt::new(
        launch,
        declaration.clone(),
        summary.ledger.clone(),
        summary.sample_stream_digest,
    )
    .expect("receipt")
}

fn declaration() -> ExecutedUncertaintyDeclaration {
    ExecutedUncertaintyDeclaration::from_condition(
        &stream_condition(),
        Digest::from_bytes([0xcd; 32]),
        GOLDEN_RUN_SEED,
    )
    .expect("declaration")
}

/// Plays the executor: produces the exact stream the declaration requires.
fn run(declaration: &ExecutedUncertaintyDeclaration, count: u64) -> Vec<ExecutedSample> {
    let mut executor = Executor::new(declaration);
    (0..count).map(|_| executor.next()).collect()
}

struct Executor<'a> {
    declaration: &'a ExecutedUncertaintyDeclaration,
    sequence: u64,
    last_accepted: Option<[u32; EXECUTED_ACTUATOR_LANE_COUNT]>,
    interval_index: u64,
    interval_position: u32,
    interval_first_sequence: u64,
    decisions: Vec<bool>,
}

impl<'a> Executor<'a> {
    fn new(declaration: &'a ExecutedUncertaintyDeclaration) -> Self {
        Self {
            declaration,
            sequence: 0,
            last_accepted: None,
            interval_index: 0,
            interval_position: 0,
            interval_first_sequence: 0,
            decisions: Vec::new(),
        }
    }

    fn next(&mut self) -> ExecutedSample {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        let sample = ExecutedSample {
            sequence,
            global_sample_sequence: sequence,
            simulator_timestamp_us: sequence.wrapping_mul(10_000),
            sensor: Some(self.sensor(sequence)),
            actuator: Some(self.actuator(sequence)),
            constraints: ExecutedConstraintFlags::default(),
            hover: ExecutedHoverInitialization {
                baseline_force_bits: BASELINE_FORCE_BITS,
                effective_force_bits: derivation::scaled_hover_force(
                    BASELINE_FORCE_BITS,
                    self.declaration.hover_scale_basis_points,
                ),
                scale_basis_points: self.declaration.hover_scale_basis_points,
                estimator_disabled: true,
                kernel_config_hash: 0x0102_0304_0506_0708,
            },
            send: ExecutedSendEvidence {
                attempted: true,
                succeeded: true,
                echoed_timestamp_us: sequence.wrapping_mul(10_000),
                lockstep: true,
            },
            armed: true,
        };
        sample
    }

    fn sensor(&self, sequence: u64) -> ExecutedSensorApplication {
        let presence_mask = 0b1001_u16;
        let mut raw = [None; EXECUTED_SENSOR_LANE_COUNT];
        #[allow(clippy::cast_precision_loss)]
        let reading = 1.0_f32 + (sequence % 7) as f32;
        raw[0] = Some(reading.to_bits());
        raw[3] = Some(0x4000_0000);
        let mut effective = raw;
        let mut update_buckets = [None; EXECUTED_SENSOR_LANE_COUNT];
        let mut changed_mask = 0_u16;
        for declared in &self.declaration.sensor_lanes {
            let lane = usize::from(declared.lane_tag);
            let bucket = sequence / u64::from(declared.update_interval_samples);
            update_buckets[lane] = Some(bucket);
            let offset = derivation::sensor_offset(
                self.declaration.condition_digest,
                self.declaration.run_seed,
                declared.lane_tag,
                bucket,
                f32::from_bits(declared.peak_amplitude_bits),
            );
            let value = f32::from_bits(raw[lane].expect("declared lane value"));
            let applied = (value + offset).to_bits();
            effective[lane] = Some(applied);
            if applied != raw[lane].expect("declared lane value") {
                changed_mask |= 1 << declared.lane_tag;
            }
        }
        ExecutedSensorApplication {
            raw_digest: derivation::sensor_sample_digest(presence_mask, &raw),
            effective_digest: derivation::sensor_sample_digest(presence_mask, &effective),
            presence_mask,
            changed_mask,
            update_buckets,
            raw_value_bits: raw,
            effective_value_bits: effective,
        }
    }

    fn actuator(&mut self, sequence: u64) -> ExecutedActuatorApplication {
        let mut requested = [0_u32; EXECUTED_ACTUATOR_LANE_COUNT];
        #[allow(clippy::cast_precision_loss)]
        let command = 0.1_f32 * ((sequence % 5) as f32 + 1.0);
        for lane in requested.iter_mut().take(usize::from(LANE_COUNT)) {
            *lane = command.to_bits();
        }
        let mut scaled = requested;
        for lane in scaled.iter_mut().take(usize::from(LANE_COUNT)) {
            *lane =
                derivation::scaled_authority(*lane, self.declaration.authority_scale_basis_points);
        }
        self.command(requested, scaled, sequence)
    }

    fn command(
        &mut self,
        requested: [u32; EXECUTED_ACTUATOR_LANE_COUNT],
        scaled: [u32; EXECUTED_ACTUATOR_LANE_COUNT],
        sequence: u64,
    ) -> ExecutedActuatorApplication {
        let base = ExecutedActuatorApplication {
            requested_lane_bits: requested,
            authority_scaled_lane_bits: scaled,
            effective_lane_bits: scaled,
            lane_count: LANE_COUNT,
            eligibility: ExecutedEligibility::Eligible,
            prime: false,
            interval_epoch: None,
            interval_index: None,
            interval_position: None,
            interval_identity: None,
            selected_hold: false,
            applied_hold: false,
            interval_complete: false,
        };
        if self.last_accepted.is_none() {
            self.last_accepted = Some(scaled);
            return ExecutedActuatorApplication {
                prime: true,
                ..base
            };
        }
        self.hold(base, scaled, sequence)
    }

    fn hold(
        &mut self,
        base: ExecutedActuatorApplication,
        scaled: [u32; EXECUTED_ACTUATOR_LANE_COUNT],
        sequence: u64,
    ) -> ExecutedActuatorApplication {
        if self.interval_position == 0 {
            self.interval_first_sequence = sequence;
            self.decisions = derivation::hold_schedule(
                self.declaration.condition_digest,
                self.declaration.run_seed,
                0,
                self.interval_index,
                sequence,
                self.declaration.command_hold.expect("declared hold"),
            )
            .expect("hold schedule");
        }
        let position = self.interval_position;
        let selected = self.decisions[usize::try_from(position).expect("position")];
        let effective = if selected {
            self.last_accepted.expect("accepted command")
        } else {
            self.last_accepted = Some(scaled);
            scaled
        };
        let identity = derivation::interval_identity(
            self.declaration.condition_digest,
            self.declaration.run_seed,
            0,
            self.interval_index,
            self.interval_first_sequence,
        );
        let complete = position.wrapping_add(1) >= HOLD_INTERVAL;
        if complete {
            self.interval_index = self.interval_index.wrapping_add(1);
            self.interval_position = 0;
        } else {
            self.interval_position = position.wrapping_add(1);
        }
        ExecutedActuatorApplication {
            effective_lane_bits: effective,
            interval_epoch: Some(0),
            interval_index: Some(base_index(self.interval_index, complete)),
            interval_position: Some(position),
            interval_identity: Some(identity),
            selected_hold: selected,
            applied_hold: selected,
            interval_complete: complete,
            ..base
        }
    }
}

const fn base_index(next_index: u64, complete: bool) -> u64 {
    if complete {
        next_index.wrapping_sub(1)
    } else {
        next_index
    }
}
