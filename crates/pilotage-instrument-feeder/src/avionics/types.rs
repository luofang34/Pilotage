//! The ingress vocabulary: sample, configuration, counter, and
//! snapshot types shared with every shell boundary.

use crate::stamp::RawStamp;

/// Whether independently acquired groups form one coherent snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Coherence {
    /// Too few stamped groups are present to establish coherence.
    #[default]
    Insufficient,
    /// Groups share a source stream and meet the skew budget.
    Coherent,
    /// Groups exceed the acquisition-time skew budget.
    ExcessiveSkew,
}

/// Coherence verdict plus the measured skew, when measurable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoherenceReport {
    /// The verdict.
    pub status: Coherence,
    /// Acquisition-time skew in nanoseconds, when both groups share a
    /// stream.
    pub skew_nanos: Option<u64>,
}

/// How an unseen incarnation is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IncarnationPolicy {
    /// The first incarnation is pinned; any other is refused.
    #[default]
    PinFirst,
    /// A never-seen incarnation is authorized as a source reset
    /// (simulation restarts); already-seen ones stay refused replays.
    SimAcceptUnseen,
}

/// Wrap-counted refusal and transition counters, mirroring the wire
/// admission vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IngressCounters {
    /// Publications carrying an already-seen epoch/sequence pair.
    pub duplicates: u32,
    /// Publications serially older than the newest admitted pair.
    pub reordered: u32,
    /// Publications for a vehicle other than the configured one.
    pub wrong_vehicle: u32,
    /// Stamps from a source other than the pinned identity.
    pub wrong_source: u32,
    /// Stamps refused under the pin-first incarnation policy.
    pub wrong_incarnation: u32,
    /// Stamps replaying an incarnation already retired.
    pub old_incarnation: u32,
    /// Authorized incarnation changes (source restarts).
    pub incarnation_transitions: u32,
    /// Incarnations dropped because the seen-set was full.
    pub incarnation_capacity: u32,
    /// Stamps carrying an epoch older than the current one.
    pub old_epoch: u32,
    /// Authorized epoch advances treated as source resets.
    pub source_resets: u32,
    /// Stamps that failed shape or role validation.
    pub invalid_stamps: u32,
    /// Serial-distance gaps observed between admitted sequences.
    pub sequence_gaps: u32,
    /// Group pairs beyond the configured skew budget.
    pub excessive_skew: u32,
    /// Acquisition timestamps that moved backwards within a stream.
    pub time_regressions: u32,
    /// Clock-domain changes observed within a pinned stream.
    pub clock_changes: u32,
}

/// Attitude estimate group as the wire delivers it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttitudeGroup {
    /// Body→NED quaternion (w, x, y, z).
    pub quat: [f32; 4],
    /// Body rates (p, q, r), radians/second.
    pub rates: [f32; 3],
    /// Arm-state code riding the estimate.
    pub arm_state: u32,
}

/// Kinematics estimate group as the wire delivers it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KinematicsGroup {
    /// Position NED, meters.
    pub pos_ned: [f32; 3],
    /// Velocity NED, meters/second.
    pub vel_ned: [f32; 3],
    /// Arm-state code riding the estimate.
    pub arm_state: u32,
}

/// One decoded avionics publication: up to three independently stamped
/// groups plus the estimator's declared trust.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvionicsSample {
    /// Vehicle the publication claims to describe.
    pub vehicle_id: u64,
    /// Attitude payload (applied only when its stamp is admitted).
    pub attitude: AttitudeGroup,
    /// Kinematics payload (applied only when its stamp is admitted).
    pub kinematics: KinematicsGroup,
    /// Declared validity flags (bits 0–1 attitude, 2–3 kinematics).
    pub valid_flags: u32,
    /// Declared estimate quality (2 = unusable).
    pub quality: u32,
    /// Stamp for the attitude group, when the lane carried one.
    pub attitude_stamp: Option<RawStamp>,
    /// Stamp for the kinematics group, when the lane carried one.
    pub kinematics_stamp: Option<RawStamp>,
    /// Stamp for the estimator-status group, when the lane carried one.
    pub estimator_status_stamp: Option<RawStamp>,
}

/// One accepted group with its stamp and caller-clock age.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroupSnapshot<T> {
    /// The accepted payload.
    pub data: T,
    /// The stamp that admitted it.
    pub stamp: RawStamp,
    /// Milliseconds since acceptance on the caller's clock.
    pub age_ms: f64,
}

/// Configuration for one [`super::AvionicsIngress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngressConfig {
    /// The vehicle whose publications this ingress accepts.
    pub vehicle_id: u64,
    /// Pre-pinned source id, or `None` to pin the first seen.
    pub source_id: Option<u64>,
    /// Pre-pinned incarnation, or `None` to pin the first seen.
    pub incarnation: Option<[u8; 16]>,
    /// Unseen-incarnation policy.
    pub incarnation_policy: IncarnationPolicy,
    /// Remembered-incarnation bound under
    /// [`IncarnationPolicy::SimAcceptUnseen`]; clamped to the fixed
    /// [`super::MAX_SEEN_INCARNATIONS`] storage.
    pub maximum_seen_incarnations: usize,
    /// Acquisition-skew budget for pairing and coherence, nanoseconds.
    pub maximum_skew_nanos: u64,
}

/// The current admitted state and trust of one ingress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IngressSnapshot {
    /// Wrapping generation advanced only when admitted state changed.
    pub generation: u32,
    /// The pinned source id, once seen.
    pub source_id: Option<u64>,
    /// The pinned incarnation, once seen.
    pub incarnation: Option<[u8; 16]>,
    /// The current source epoch, once seen.
    pub epoch: Option<u32>,
    /// The admitted attitude group.
    pub attitude: Option<GroupSnapshot<AttitudeGroup>>,
    /// The admitted kinematics group.
    pub kinematics: Option<GroupSnapshot<KinematicsGroup>>,
    /// The admitted estimator-status stamp (its payload is trust).
    pub estimator_status: Option<GroupSnapshot<()>>,
    /// Authorized validity flags after regime pairing.
    pub valid_flags: u32,
    /// Authorized quality after regime pairing (2 = unusable).
    pub quality: u32,
    /// Attitude/kinematics acquisition coherence.
    pub coherence: CoherenceReport,
}
