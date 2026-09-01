//! One executor that speaks the trace protocol over a real loopback path.
//!
//! It answers exactly what the declaration requires, so a test that changes
//! one value changes only that value and the refusal names it.

use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;

use flight_tune::{ExecutedUncertaintyDeclaration, derivation};

use super::super::frame;
use super::super::protocol::{
    TuningActuatorApplication, TuningActuatorEligibility, TuningConstraintFlags,
    TuningControlObservation, TuningFrameType, TuningHandshake, TuningHoverEstimatorMode,
    TuningHoverInitialization, TuningObservationAck, TuningPerturbationCapability, TuningReady,
    TuningSendEvidence, TuningSensorApplication,
};

pub(super) const BASELINE_FORCE_BITS: u32 = 0x3f00_0000;
pub(super) const KERNEL_HASH: u64 = 0x0102_0304_0506_0708;
pub(super) const LANE_COUNT: u8 = 4;
const MANIFEST_DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const KERNEL_HASH_TEXT: &str = "0102030405060708";

/// What one executor states other than the truth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Fault {
    /// The executor answers exactly.
    None,
    /// The executor loaded another condition document.
    ChangedConditionDigest,
    /// The executor drew from another seed.
    ChangedRunSeed,
    /// The executor supplies fewer capabilities than the condition needs.
    MissingCapability,
    /// The executor states an active online hover estimator.
    ActiveHoverEstimator,
    /// The executor skips one sample sequence.
    SequenceGap,
    /// The executor states a value it did not derive.
    ChangedSensorValue,
    /// The executor stops inside a decision interval.
    ShortRun,
    /// The executor states another hover force part way through.
    MovedHoverForce,
}

/// One executor run over the trace path.
pub(super) struct Executor {
    declaration: ExecutedUncertaintyDeclaration,
    artifact_path: PathBuf,
    samples: u64,
    fault: Fault,
    last_accepted: Option<[u32; 16]>,
    interval_index: u64,
    interval_position: u32,
    interval_first_sequence: u64,
    decisions: Vec<bool>,
}

impl Executor {
    pub(super) fn new(
        declaration: &ExecutedUncertaintyDeclaration,
        artifact_path: PathBuf,
        samples: u64,
        fault: Fault,
    ) -> Self {
        Self {
            declaration: declaration.clone(),
            artifact_path,
            samples,
            fault,
            last_accepted: None,
            interval_index: 0,
            interval_position: 0,
            interval_first_sequence: 0,
            decisions: Vec::new(),
        }
    }

    /// Connects, states the run, and answers every sample it is asked for.
    pub(super) fn run_blocking(mut self, endpoint: SocketAddr) -> Result<(), String> {
        let mut stream = TcpStream::connect(endpoint).map_err(|error| error.to_string())?;
        frame::write(&mut stream, &self.handshake()).map_err(|error| error.to_string())?;
        let ready: TuningReady = frame::read(&mut stream).map_err(|error| error.to_string())?;
        if ready.frame_type != TuningFrameType::AviateTuningReady {
            return Err("the launcher did not accept the run".to_owned());
        }
        let samples = if self.fault == Fault::ShortRun {
            self.samples.saturating_sub(6)
        } else {
            self.samples
        };
        for index in 0..samples {
            let observation = self.observation(index);
            frame::write(&mut stream, &observation).map_err(|error| error.to_string())?;
            let ack: TuningObservationAck =
                frame::read(&mut stream).map_err(|error| error.to_string())?;
            if ack.sequence != observation.sequence {
                return Err("the launcher answered another sample".to_owned());
            }
        }
        Ok(())
    }

    fn handshake(&self) -> TuningHandshake {
        let identity = &self.declaration;
        let mut condition_digest = identity.condition_digest.to_string();
        if self.fault == Fault::ChangedConditionDigest {
            condition_digest = "0".repeat(64);
        }
        let mut capabilities = identity
            .required_capabilities
            .iter()
            .map(|capability| match capability.as_str() {
                "actuator_authority" => TuningPerturbationCapability::ActuatorAuthority,
                "command_hold" => TuningPerturbationCapability::CommandHold,
                "hover_trim_uncertainty" => TuningPerturbationCapability::HoverTrimUncertainty,
                _ => TuningPerturbationCapability::SensorPerturbation,
            })
            .collect::<Vec<_>>();
        if self.fault == Fault::MissingCapability {
            capabilities.pop();
        }
        TuningHandshake {
            frame_type: TuningFrameType::AviateTuningHandshake,
            schema_version: super::super::TUNING_TRACE_SCHEMA_VERSION,
            run_manifest_digest: MANIFEST_DIGEST.to_owned(),
            kernel_config_hash: KERNEL_HASH_TEXT.to_owned(),
            condition_artifact_path: Some(self.artifact_path.to_string_lossy().into_owned()),
            condition_artifact_sha256: Some(identity.artifact_digest.to_string()),
            condition_digest: Some(condition_digest),
            condition_run_seed: Some(match self.fault {
                Fault::ChangedRunSeed => identity.run_seed.wrapping_add(1),
                _ => identity.run_seed,
            }),
            condition_required_capabilities: Some(capabilities),
            hover_baseline_force_bits: BASELINE_FORCE_BITS,
            hover_effective_force_bits: derivation::scaled_hover_force(
                BASELINE_FORCE_BITS,
                identity.hover_scale_basis_points,
            ),
            hover_scale_basis_points: identity.hover_scale_basis_points,
            hover_estimator_mode: match self.fault {
                Fault::ActiveHoverEstimator => TuningHoverEstimatorMode::Online,
                _ => TuningHoverEstimatorMode::Disabled,
            },
            hover_kernel_config_hash: KERNEL_HASH_TEXT.to_owned(),
        }
    }

    fn observation(&mut self, index: u64) -> TuningControlObservation {
        let sequence = match self.fault {
            Fault::SequenceGap if index >= 5 => index.wrapping_add(1),
            _ => index,
        };
        TuningControlObservation {
            frame_type: TuningFrameType::AviateControlObservation,
            schema_version: super::super::TUNING_TRACE_SCHEMA_VERSION,
            sequence,
            simulator_timestamp_us: index.wrapping_mul(10_000),
            global_sample_sequence: sequence,
            sensor_application: Some(self.sensor(sequence, index)),
            actuator_application: Some(self.actuator(sequence)),
            constraint_flags: TuningConstraintFlags::default(),
            hover_initialization: self.hover(index),
            send: TuningSendEvidence {
                reply_attempted: true,
                reply_succeeded: true,
                echoed_timestamp_us: index.wrapping_mul(10_000),
                lockstep: true,
            },
            armed: true,
        }
    }

    fn hover(&self, index: u64) -> TuningHoverInitialization {
        let baseline = if self.fault == Fault::MovedHoverForce && index >= 5 {
            0x3f40_0000
        } else {
            BASELINE_FORCE_BITS
        };
        TuningHoverInitialization {
            baseline_force_bits: baseline,
            effective_force_bits: derivation::scaled_hover_force(
                baseline,
                self.declaration.hover_scale_basis_points,
            ),
            scale_basis_points: self.declaration.hover_scale_basis_points,
            kernel_config_hash: KERNEL_HASH,
        }
    }

    fn sensor(&self, sequence: u64, index: u64) -> TuningSensorApplication {
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
            let mut applied = (value + offset).to_bits();
            if self.fault == Fault::ChangedSensorValue && index >= 5 {
                applied = value.to_bits();
            }
            effective[lane] = Some(applied);
            if applied != raw[lane].unwrap_or(0) {
                changed_mask |= 1 << declared.lane_tag;
            }
        }
        TuningSensorApplication {
            raw_digest: *derivation::sensor_sample_digest(presence_mask, &raw).as_bytes(),
            effective_digest: *derivation::sensor_sample_digest(presence_mask, &effective)
                .as_bytes(),
            presence_mask,
            changed_mask,
            update_buckets,
            raw_value_bits: raw,
            effective_value_bits: effective,
        }
    }

    fn actuator(&mut self, sequence: u64) -> TuningActuatorApplication {
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
        let base = TuningActuatorApplication {
            requested_lane_bits: requested,
            authority_scaled_lane_bits: scaled,
            effective_lane_bits: scaled,
            lane_count: LANE_COUNT,
            actuator_answer_armed: true,
            kernel_fallback_mask: 0,
            eligibility: TuningActuatorEligibility::Eligible,
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
            return TuningActuatorApplication {
                prime: true,
                ..base
            };
        }
        self.hold(base, scaled, sequence)
    }

    fn hold(
        &mut self,
        base: TuningActuatorApplication,
        scaled: [u32; 16],
        sequence: u64,
    ) -> TuningActuatorApplication {
        let hold = self
            .declaration
            .command_hold
            .unwrap_or(flight_tune::DeclaredCommandHold {
                fraction_basis_points: 1_000,
                decision_interval_samples: 10,
            });
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
            .unwrap_or_default();
        }
        let position = self.interval_position;
        let selected = self
            .decisions
            .get(usize::try_from(position).unwrap_or(0))
            .copied()
            .unwrap_or(false);
        let effective = if selected {
            self.last_accepted.unwrap_or(scaled)
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
        let complete = position.wrapping_add(1) >= hold.decision_interval_samples;
        if complete {
            self.interval_index = self.interval_index.wrapping_add(1);
            self.interval_position = 0;
        } else {
            self.interval_position = position.wrapping_add(1);
        }
        TuningActuatorApplication {
            effective_lane_bits: effective,
            interval_epoch: Some(0),
            interval_index: Some(index),
            interval_position: Some(position),
            interval_identity: Some(*identity.as_bytes()),
            selected_hold: selected,
            applied_hold: selected,
            interval_complete: complete,
            ..base
        }
    }
}
