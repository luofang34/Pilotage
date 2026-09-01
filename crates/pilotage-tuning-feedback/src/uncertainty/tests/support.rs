//! One executor stream the attacks start from.
//!
//! The stream is built to satisfy the core relation and is sealed through
//! the core receipt, so an attack below changes a run that really verified
//! rather than one this file only claims would.

use flight_tune::{
    ConditionSet, Digest, ExecutedActuatorApplication, ExecutedConstraintFlags,
    ExecutedEligibility, ExecutedHoverInitialization, ExecutedLaunchIdentity, ExecutedSample,
    ExecutedSendEvidence, ExecutedSensorApplication, ExecutedStream,
    ExecutedUncertaintyDeclaration, ExecutedUncertaintyReceipt, derivation,
};

pub(super) const RUN_SEED: u64 = 0x1112_1314_1516_1718;
const BASELINE_FORCE_BITS: u32 = 0x3f00_0000;
const HOLD_INTERVAL: u32 = 10;
const LANE_COUNT: u8 = 4;
const ARTIFACT_FILL: u8 = 0xcd;
const RUN_INTENT_FILL: u8 = 0x21;

const CONDITION: &str = concat!(
    r#"{"schema_version":4,"id":"executed-uncertainty-verified","revision":1,"seed":11,"#,
    r#""wind":{"steady":{"speed_mps":0.0,"direction_deg":0.0},"gusts":[],"#,
    r#""turbulence":{"kind":"none"}},"#,
    r#""timing":{"estimate_delay_ns":0,"update_jitter":{"kind":"none"}},"#,
    r#""sensor":{"kind":"bounded_noise","lanes":["#,
    r#"{"sensor":"accelerometer","axis":"x","peak_amplitude_mps2":2.0,"#,
    r#""update_interval_samples":2}]},"#,
    r#""actuator":{"authority_scale_basis_points":12000,"#,
    r#""command_loss":{"kind":"seeded_zero_order_hold","fraction_basis_points":1000,"#,
    r#""decision_interval_samples":10}},"#,
    r#""controller_initialization":{"hover_thrust_force":{"kind":"scale_baseline","#,
    r#""scale_basis_points":9000}},"#,
    r#""plant":{"payload_mass_delta_kg":0.0,"longitudinal_cg_offset_m":0.0,"#,
    r#""lateral_cg_offset_m":0.0,"hover_thrust_expectation":{"kind":"measured_weight_ratio"}}}"#,
);

/// One run the core sealed after verifying every sample it carries.
pub(super) struct SealedRun {
    pub(super) receipt: ExecutedUncertaintyReceipt,
    pub(super) samples: Vec<ExecutedSample>,
}

/// Seals one complete run of the given length.
pub(super) fn sealed(count: u64) -> SealedRun {
    let declaration = declaration();
    let samples = stream(&declaration, count);
    let mut verified = ExecutedStream::open(&declaration).expect("open the core stream");
    for sample in &samples {
        verified.accept(sample).expect("the core stream verifies");
    }
    let summary = verified.close().expect("the core stream seals");
    let launch = ExecutedLaunchIdentity::new(
        Digest::from_bytes([RUN_INTENT_FILL; 32]),
        declaration.artifact_digest,
        declaration.condition_digest,
        declaration.run_seed,
        declaration.required_capabilities.clone(),
        3,
    )
    .expect("launch identity");
    let receipt = ExecutedUncertaintyReceipt::new(
        launch,
        declaration,
        summary.ledger,
        summary.sample_stream_digest,
    )
    .expect("seal the receipt");
    SealedRun { receipt, samples }
}

pub(super) fn declaration() -> ExecutedUncertaintyDeclaration {
    let condition = ConditionSet::from_json(CONDITION.as_bytes()).expect("condition");
    ExecutedUncertaintyDeclaration::from_condition(
        &condition,
        Digest::from_bytes([ARTIFACT_FILL; 32]),
        RUN_SEED,
    )
    .expect("declaration")
}

fn stream(declaration: &ExecutedUncertaintyDeclaration, count: u64) -> Vec<ExecutedSample> {
    let mut executor = Executor {
        declaration: declaration.clone(),
        last_accepted: None,
        interval_index: 0,
        interval_position: 0,
        interval_first_sequence: 0,
        decisions: Vec::new(),
    };
    (0..count).map(|index| executor.sample(index)).collect()
}

struct Executor {
    declaration: ExecutedUncertaintyDeclaration,
    last_accepted: Option<[u32; 16]>,
    interval_index: u64,
    interval_position: u32,
    interval_first_sequence: u64,
    decisions: Vec<bool>,
}

impl Executor {
    fn sample(&mut self, sequence: u64) -> ExecutedSample {
        ExecutedSample {
            sequence,
            global_sample_sequence: sequence,
            simulator_timestamp_us: sequence.wrapping_mul(10_000),
            sensor: Some(self.sensor(sequence)),
            actuator: Some(self.actuator(sequence)),
            constraints: ExecutedConstraintFlags {
                injection_clamp: false,
                invalid_actuator_count: false,
                missing_actuator_answer: false,
                collective_rate: false,
                mean_ceiling: false,
                lane_ceiling: false,
                ground_squeeze: false,
                trace_failure: false,
            },
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
        }
    }

    fn sensor(&self, sequence: u64) -> ExecutedSensorApplication {
        let presence_mask = 0b1001_u16;
        let mut raw = [None; 12];
        #[allow(clippy::cast_precision_loss)]
        let reading = 1.0_f32 + (sequence % 7) as f32;
        raw[0] = Some(reading.to_bits());
        raw[3] = Some(0x4000_0000);
        let mut effective = raw;
        let mut update_buckets = [None; 12];
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
            let value = f32::from_bits(raw[lane].unwrap_or(0));
            let applied = (value + offset).to_bits();
            effective[lane] = Some(applied);
            if applied != raw[lane].unwrap_or(0) {
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
        let mut requested = [0_u32; 16];
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
        scaled: [u32; 16],
        sequence: u64,
    ) -> ExecutedActuatorApplication {
        let hold = self.declaration.command_hold.expect("declared hold");
        if self.interval_position == 0 {
            self.interval_first_sequence = sequence;
            self.decisions = derivation::hold_schedule(
                self.declaration.condition_digest,
                self.declaration.run_seed,
                0,
                self.interval_index,
                sequence,
                hold,
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
        let index = self.interval_index;
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
            interval_index: Some(index),
            interval_position: Some(position),
            interval_identity: Some(identity),
            selected_hold: selected,
            applied_hold: selected,
            interval_complete: complete,
            ..base
        }
    }
}
