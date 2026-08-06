//! Marshalling shapes between the script boundary and the feeder crate:
//! camelCase objects with BigInt u64s and 32-hex incarnations on the
//! JavaScript side, wire-coded plain data on the Rust side.

use pilotage_instrument_feeder::RawStamp;
use pilotage_instrument_feeder::avionics::{
    AttitudeGroup, AvionicsSample, Coherence, IngressCounters, IngressSnapshot, KinematicsGroup,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;

pub(super) fn serialize<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let serializer = serde_wasm_bindgen::Serializer::new()
        .serialize_large_number_types_as_bigints(true)
        .serialize_missing_as_null(true);
    value
        .serialize(&serializer)
        .map_err(|error| JsValue::from_str(&format!("serialize: {error}")))
}

pub(super) fn parse_incarnation(hex: &str) -> Result<[u8; 16], &'static str> {
    let raw = hex.as_bytes();
    if raw.len() != 32 {
        return Err("incarnation must be 32 hex characters");
    }
    let digit = |byte: u8| -> Result<u8, &'static str> {
        match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => Err("incarnation must be lowercase hex"),
        }
    };
    let mut out = [0u8; 16];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = (digit(raw[index * 2])? << 4) | digit(raw[index * 2 + 1])?;
    }
    Ok(out)
}

fn incarnation_hex(bytes: [u8; 16]) -> String {
    use core::fmt::Write as _;
    let mut out = String::new();
    for byte in bytes {
        write!(out, "{byte:02x}").ok();
    }
    out
}

/// A measurement stamp as the script decodes it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JsStamp {
    pub role: u8,
    pub integrity: u8,
    pub source_id: u64,
    pub source_incarnation: String,
    pub source_epoch: u32,
    pub sequence: u32,
    pub acquired_at_nanos: u64,
    pub clock: u8,
}

impl TryFrom<JsStamp> for RawStamp {
    type Error = &'static str;

    fn try_from(stamp: JsStamp) -> Result<Self, Self::Error> {
        Ok(RawStamp {
            role: stamp.role,
            integrity: stamp.integrity,
            source_id: stamp.source_id,
            incarnation: parse_incarnation(&stamp.source_incarnation)?,
            epoch: stamp.source_epoch,
            sequence: stamp.sequence,
            acquired_at_ns: stamp.acquired_at_nanos,
            clock: stamp.clock,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsStampOut {
    role: u8,
    integrity: u8,
    source_id: u64,
    source_incarnation: String,
    source_epoch: u32,
    sequence: u32,
    acquired_at_nanos: u64,
    clock: u8,
}

impl From<RawStamp> for JsStampOut {
    fn from(stamp: RawStamp) -> Self {
        Self {
            role: stamp.role,
            integrity: stamp.integrity,
            source_id: stamp.source_id,
            source_incarnation: incarnation_hex(stamp.incarnation),
            source_epoch: stamp.epoch,
            sequence: stamp.sequence,
            acquired_at_nanos: stamp.acquired_at_ns,
            clock: stamp.clock,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsQuat {
    w: f32,
    x: f32,
    y: f32,
    z: f32,
}

/// One decoded avionics publication as the script hands it over.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JsSample {
    vehicle_id: u64,
    quat: JsQuat,
    rates: [f32; 3],
    pos_ned: [f32; 3],
    vel_ned: [f32; 3],
    arm_state: u32,
    valid_flags: u32,
    quality: u32,
    #[serde(default)]
    attitude_stamp: Option<JsStamp>,
    #[serde(default)]
    kinematics_stamp: Option<JsStamp>,
    #[serde(default)]
    estimator_status_stamp: Option<JsStamp>,
}

impl TryFrom<JsSample> for AvionicsSample {
    type Error = &'static str;

    fn try_from(sample: JsSample) -> Result<Self, Self::Error> {
        let convert = |stamp: Option<JsStamp>| -> Result<Option<RawStamp>, &'static str> {
            stamp.map(RawStamp::try_from).transpose()
        };
        Ok(AvionicsSample {
            vehicle_id: sample.vehicle_id,
            attitude: AttitudeGroup {
                quat: [sample.quat.w, sample.quat.x, sample.quat.y, sample.quat.z],
                rates: sample.rates,
                arm_state: sample.arm_state,
            },
            kinematics: KinematicsGroup {
                pos_ned: sample.pos_ned,
                vel_ned: sample.vel_ned,
                arm_state: sample.arm_state,
            },
            valid_flags: sample.valid_flags,
            quality: sample.quality,
            attitude_stamp: convert(sample.attitude_stamp)?,
            kinematics_stamp: convert(sample.kinematics_stamp)?,
            estimator_status_stamp: convert(sample.estimator_status_stamp)?,
        })
    }
}

/// The ingress refusal counters in the script's camelCase vocabulary.
/// Field meanings are the feeder crate's `IngressCounters`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JsIngressCounters {
    /// Already-seen epoch/sequence pairs.
    pub duplicates: u32,
    /// Serially older than the newest admitted pair.
    pub reordered: u32,
    /// Publications for another vehicle.
    pub wrong_vehicle: u32,
    /// Stamps from an unpinned source.
    pub wrong_source: u32,
    /// Refused under the pin-first incarnation policy.
    pub wrong_incarnation: u32,
    /// Replays of a retired incarnation.
    pub old_incarnation: u32,
    /// Authorized incarnation changes.
    pub incarnation_transitions: u32,
    /// Incarnations dropped at seen-set capacity.
    pub incarnation_capacity: u32,
    /// Epochs older than the current one.
    pub old_epoch: u32,
    /// Authorized epoch advances (source resets).
    pub source_resets: u32,
    /// Stamps failing shape or role validation.
    pub invalid_stamps: u32,
    /// Serial-distance gaps between admitted sequences.
    pub sequence_gaps: u32,
    /// Group pairs beyond the skew budget.
    pub excessive_skew: u32,
    /// Backwards acquisition timestamps within a stream.
    pub time_regressions: u32,
    /// Clock-domain changes within a pinned stream.
    pub clock_changes: u32,
}

impl From<IngressCounters> for JsIngressCounters {
    fn from(counters: IngressCounters) -> Self {
        Self {
            duplicates: counters.duplicates,
            reordered: counters.reordered,
            wrong_vehicle: counters.wrong_vehicle,
            wrong_source: counters.wrong_source,
            wrong_incarnation: counters.wrong_incarnation,
            old_incarnation: counters.old_incarnation,
            incarnation_transitions: counters.incarnation_transitions,
            incarnation_capacity: counters.incarnation_capacity,
            old_epoch: counters.old_epoch,
            source_resets: counters.source_resets,
            invalid_stamps: counters.invalid_stamps,
            sequence_gaps: counters.sequence_gaps,
            excessive_skew: counters.excessive_skew,
            time_regressions: counters.time_regressions,
            clock_changes: counters.clock_changes,
        }
    }
}

pub(super) fn coherence_str(coherence: Coherence) -> &'static str {
    match coherence {
        Coherence::Insufficient => "insufficient",
        Coherence::Coherent => "coherent",
        Coherence::ExcessiveSkew => "excessive-skew",
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsCoherence {
    status: &'static str,
    skew_nanos: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsAttitudeSnapshot {
    quat: JsQuatOut,
    rates: [f32; 3],
    arm_state: u32,
    stamp: JsStampOut,
    age_ms: f64,
}

#[derive(Debug, Serialize)]
struct JsQuatOut {
    w: f32,
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsKinematicsSnapshot {
    pos_ned: [f32; 3],
    vel_ned: [f32; 3],
    arm_state: u32,
    stamp: JsStampOut,
    age_ms: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsStatusSnapshot {
    stamp: JsStampOut,
    age_ms: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsSnapshot {
    generation: u32,
    source_id: Option<u64>,
    source_incarnation: Option<String>,
    source_epoch: Option<u32>,
    attitude: Option<JsAttitudeSnapshot>,
    kinematics: Option<JsKinematicsSnapshot>,
    estimator_status: Option<JsStatusSnapshot>,
    valid_flags: u32,
    quality: u32,
    coherence: JsCoherence,
}

pub(super) fn snapshot_js(snapshot: &IngressSnapshot) -> Result<JsValue, JsValue> {
    serialize(&JsSnapshot {
        generation: snapshot.generation,
        source_id: snapshot.source_id,
        source_incarnation: snapshot.incarnation.map(incarnation_hex),
        source_epoch: snapshot.epoch,
        attitude: snapshot.attitude.map(|group| JsAttitudeSnapshot {
            quat: JsQuatOut {
                w: group.data.quat[0],
                x: group.data.quat[1],
                y: group.data.quat[2],
                z: group.data.quat[3],
            },
            rates: group.data.rates,
            arm_state: group.data.arm_state,
            stamp: group.stamp.into(),
            age_ms: group.age_ms,
        }),
        kinematics: snapshot.kinematics.map(|group| JsKinematicsSnapshot {
            pos_ned: group.data.pos_ned,
            vel_ned: group.data.vel_ned,
            arm_state: group.data.arm_state,
            stamp: group.stamp.into(),
            age_ms: group.age_ms,
        }),
        estimator_status: snapshot.estimator_status.map(|group| JsStatusSnapshot {
            stamp: group.stamp.into(),
            age_ms: group.age_ms,
        }),
        valid_flags: snapshot.valid_flags,
        quality: snapshot.quality,
        coherence: JsCoherence {
            status: coherence_str(snapshot.coherence.status),
            skew_nanos: snapshot.coherence.skew_nanos,
        },
    })
}

mod lanes;
pub(super) use lanes::{
    JsFcReport, JsFcView, JsGuidanceSample, JsNavDiagnostics, JsNavGroup, JsNavSnapshot,
    JsNavSnapshotIn, JsTurnDeclaration,
};
