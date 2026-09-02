//! The trace frames the executor and this launcher exchange.
//!
//! The shapes here match the executing contract by value: every serialized
//! name, every tag, and every encoding is the one the executor writes. A
//! field the launcher does not answer for is not mirrored, so the observation
//! carries the projection this repository is accountable for rather than the
//! executor's complete record.

use serde::{Deserialize, Serialize};

/// The frames the trace protocol names.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TuningFrameType {
    /// The executor states its run identities.
    AviateTuningHandshake,
    /// The launcher accepts those identities.
    AviateTuningReady,
    /// The executor states one sample.
    AviateControlObservation,
    /// The launcher accepts one sample.
    AviateTuningObservationAck,
}

/// One Aviate-owned perturbation capability.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningPerturbationCapability {
    /// Scale eligible actuator commands.
    ActuatorAuthority,
    /// Apply a deterministic command hold.
    CommandHold,
    /// Scale the controller hover-force initialization.
    HoverTrimUncertainty,
    /// Apply deterministic bounded sensor perturbations.
    SensorPerturbation,
}

/// The online hover-estimator state for one run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningHoverEstimatorMode {
    /// The estimator can write over the hover force.
    Online,
    /// No online component changes the hover force.
    Disabled,
    /// The estimator keeps one fixed value.
    Frozen,
}

/// The run identities the executor states before its first sample.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TuningHandshake {
    /// The frame kind.
    #[serde(rename = "type")]
    pub frame_type: TuningFrameType,
    /// The trace protocol version.
    pub schema_version: u16,
    /// The identity of the exact run manifest text.
    pub run_manifest_digest: String,
    /// The identity of the resolved kernel.
    pub kernel_config_hash: String,
    /// The path of the loaded condition artifact.
    #[serde(default)]
    pub condition_artifact_path: Option<String>,
    /// The identity of the exact condition artifact bytes.
    #[serde(default)]
    pub condition_artifact_sha256: Option<String>,
    /// The identity of the canonical condition document.
    #[serde(default)]
    pub condition_digest: Option<String>,
    /// The seed for every deterministic decision in this run.
    #[serde(default)]
    pub condition_run_seed: Option<u64>,
    /// The capabilities the loaded condition needs, in ascending name order.
    #[serde(default)]
    pub condition_required_capabilities: Option<Vec<TuningPerturbationCapability>>,
    /// The preset hover-force baseline.
    pub hover_baseline_force_bits: u32,
    /// The hover force the controller was built with.
    pub hover_effective_force_bits: u32,
    /// The applied hover-force scale.
    pub hover_scale_basis_points: u16,
    /// The online hover-estimator state for this run.
    pub hover_estimator_mode: TuningHoverEstimatorMode,
    /// The kernel identity that carries the hover force.
    pub hover_kernel_config_hash: String,
}

/// The launcher's acceptance of one handshake.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TuningReady {
    /// The frame kind.
    #[serde(rename = "type")]
    pub frame_type: TuningFrameType,
    /// The trace protocol version.
    pub schema_version: u16,
    /// The run manifest identity the handshake stated.
    pub run_manifest_digest: String,
}

/// The launcher's acceptance of one sample.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TuningObservationAck {
    /// The frame kind.
    #[serde(rename = "type")]
    pub frame_type: TuningFrameType,
    /// The trace protocol version.
    pub schema_version: u16,
    /// The run manifest identity the handshake stated.
    pub run_manifest_digest: String,
    /// The sequence this acceptance answers.
    pub sequence: u64,
}

/// Sensor values before and after one deterministic perturbation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TuningSensorApplication {
    /// The identity of the values read before any change.
    pub raw_digest: [u8; 32],
    /// The identity of the values the controller received.
    pub effective_digest: [u8; 32],
    /// The lanes this sample carried, in stable lane order.
    pub presence_mask: u16,
    /// The lanes whose exact value moved.
    pub changed_mask: u16,
    /// The held-offset bucket for each configured and present lane.
    pub update_buckets: [Option<u64>; 12],
    /// The exact values read, in stable lane order.
    pub raw_value_bits: [Option<u32>; 12],
    /// The exact values the controller received, in stable lane order.
    pub effective_value_bits: [Option<u32>; 12],
}

/// Why one sample carried no actuator perturbation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TuningActuatorBypassReason {
    /// The controller supplied no actuator answer.
    MissingAnswer,
    /// The answer named a different number of lanes.
    InvalidActuatorCount,
    /// A backup path produced the command.
    Backup,
    /// A direct command reached the actuator.
    Direct,
    /// A failsafe produced the command.
    Failsafe,
    /// The controller raised a fallback lane.
    FallbackMask,
    /// The armed state changed on this sample.
    ArmTransition,
    /// The vehicle was not armed.
    Disarmed,
    /// An emergency termination produced the command.
    EmergencyTermination,
}

/// Whether one sample was eligible for an actuator perturbation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "kebab-case")]
pub enum TuningActuatorEligibility {
    /// The sample was eligible.
    Eligible,
    /// The sample bypassed every perturbation policy.
    Bypass(TuningActuatorBypassReason),
}

/// Actuator values before the plant constraints and transforms.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TuningActuatorApplication {
    /// The exact values the controller requested.
    pub requested_lane_bits: [u32; 16],
    /// The exact values after the authority scale.
    pub authority_scaled_lane_bits: [u32; 16],
    /// The exact values the plant boundary received.
    pub effective_lane_bits: [u32; 16],
    /// The number of active actuator lanes.
    pub lane_count: u8,
    /// The armed state in the actuator answer.
    pub actuator_answer_armed: bool,
    /// The controller fallback lanes this sample raised.
    pub kernel_fallback_mask: u8,
    /// Whether this sample was eligible for a perturbation.
    pub eligibility: TuningActuatorEligibility,
    /// Whether this sample only primed the accepted-command history.
    pub prime: bool,
    /// The interval epoch, which a bypass advances.
    pub interval_epoch: Option<u64>,
    /// The interval index inside the current epoch.
    pub interval_index: Option<u64>,
    /// The zero-based position inside the current interval.
    pub interval_position: Option<u32>,
    /// The identity of the current interval.
    pub interval_identity: Option<[u8; 32]>,
    /// Whether the seeded schedule selected a hold at this position.
    pub selected_hold: bool,
    /// Whether the executor applied a hold at this position.
    pub applied_hold: bool,
    /// Whether this sample completed the interval.
    pub interval_complete: bool,
}

/// The plant constraints one sample reported.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TuningConstraintFlags {
    /// A requested injection did not reach the plant unchanged.
    pub injection_clamp: bool,
    /// The answer named a different number of lanes.
    pub invalid_actuator_count: bool,
    /// The controller supplied no actuator answer.
    pub missing_actuator_answer: bool,
    /// A collective rate limit changed a lane.
    pub collective_rate: bool,
    /// A mean ceiling changed a lane.
    pub mean_ceiling: bool,
    /// A single-lane ceiling changed a lane.
    pub lane_ceiling: bool,
    /// A ground constraint changed a lane.
    pub ground_squeeze: bool,
    /// The trace path itself failed on this sample.
    pub tuning_trace_failure: bool,
}

/// The controller hover initialization one sample repeats.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TuningHoverInitialization {
    /// The preset force-domain baseline.
    pub baseline_force_bits: u32,
    /// The force-domain value the controller was built with.
    pub effective_force_bits: u32,
    /// The applied hover-force scale.
    pub scale_basis_points: u16,
    /// The resolved kernel identity that carries this value.
    pub kernel_config_hash: u64,
}

/// The one actuator send this sample attempted.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TuningSendEvidence {
    /// Whether the executor attempted the send.
    pub reply_attempted: bool,
    /// Whether the send completed.
    pub reply_succeeded: bool,
    /// The sample time the send echoed.
    pub echoed_timestamp_us: u64,
    /// Whether the send answered the sensor sample in lockstep.
    pub lockstep: bool,
}

/// One high-rate observation the launcher answers for.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct TuningControlObservation {
    /// The frame kind.
    #[serde(rename = "type")]
    pub frame_type: TuningFrameType,
    /// The trace protocol version.
    pub schema_version: u16,
    /// The gap-free sequence the trace publisher assigned.
    pub sequence: u64,
    /// The simulator sample time.
    pub simulator_timestamp_us: u64,
    /// The global sample sequence every seeded decision uses.
    pub global_sample_sequence: u64,
    /// The sensor evidence, when this sample carried sensor values.
    #[serde(default)]
    pub sensor_application: Option<TuningSensorApplication>,
    /// The actuator evidence, when this sample commanded the actuator.
    #[serde(default)]
    pub actuator_application: Option<TuningActuatorApplication>,
    /// The plant constraints this sample reported.
    pub constraint_flags: TuningConstraintFlags,
    /// The repeated controller hover initialization.
    pub hover_initialization: TuningHoverInitialization,
    /// The one send this sample attempted.
    pub send: TuningSendEvidence,
    /// The kernel armed state after this sample.
    pub armed: bool,
}
