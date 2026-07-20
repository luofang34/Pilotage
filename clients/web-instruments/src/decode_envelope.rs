//! Browser-facing decode of the datagram-channel `Envelope`, compiled from the
//! host's own `prost` types (ADR-0014), so the `TelemetrySample` shape — the
//! other wire surface the viewer reads at rate — can never drift from the
//! schema the host encodes with.
//!
//! The export mirrors the hand-written JS `decodeBareEnvelope`: it returns
//! `{ kind, message }`, where `message` is the arm the datagram channel
//! carries (a telemetry sample, a `Pong`, or a frame rejection). The bootstrap
//! stream's handshake arms are one-time and stay on the JS reader.

use pilotage_protocol::wire;
use prost::Message;
use serde::Serialize;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::wire_js::{incarnation_hex, to_js};

/// `{ kind, message }`, the browser's envelope-decode result shape.
#[derive(Serialize)]
struct Decoded<M> {
    kind: &'static str,
    message: M,
}

/// The `Pong` and `unknown` arms carry no fields the viewer reads.
#[derive(Serialize)]
struct Empty {}

/// A `FrameRejected`, of which the viewer displays only the reason code.
#[derive(Serialize)]
struct FrameRejectedMessage {
    reason: i32,
}

/// A `MeasurementStamp` in the browser gate's field vocabulary. `sourceId` and
/// `acquiredAtNanos` serialize to `BigInt`; the rest to `Number`; an
/// incarnation that is not exactly 16 bytes serializes to `null` (the browser
/// validator then rejects the group), never a truncated hex string.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stamp {
    source_id: u64,
    source_incarnation: Option<String>,
    source_epoch: u32,
    sequence: u32,
    acquired_at_nanos: u64,
    clock: i32,
    // Explicit source role; consumers gate on this, never on id ranges.
    role: i32,
    // Integrity classification of the delivering path.
    integrity: i32,
}

#[derive(Serialize, Clone, Copy)]
struct Quat {
    w: f32,
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Serialize, Clone, Copy)]
struct Attitude {
    quat: Quat,
    rates: [f32; 3],
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct Kinematics {
    pos_ned: [f32; 3],
    vel_ned: [f32; 3],
}

/// Raw avionics estimate in the exact shape the browser ingress consumes: the
/// attitude and kinematics groups are present only when their acquisition stamp
/// is, and the flattened `quat`/`rates`/`posNed`/`velNed` mirror those groups.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Avionics {
    attitude: Option<Attitude>,
    kinematics: Option<Kinematics>,
    quat: Option<Quat>,
    rates: Option<[f32; 3]>,
    pos_ned: Option<[f32; 3]>,
    vel_ned: Option<[f32; 3]>,
    valid_flags: u32,
    quality: u32,
    arm_state: u32,
    attitude_stamp: Option<Stamp>,
    kinematics_stamp: Option<Stamp>,
    estimator_status_stamp: Option<Stamp>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Pose {
    x_m: f32,
    y_m: f32,
    heading_rad: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Velocity {
    linear_x_mps: f32,
    linear_y_mps: f32,
    angular_rad_s: f32,
}

/// Simulation-truth oracle sample in the browser's shape: the simulator's
/// pose under its own provenance stamp. Kept structurally apart from
/// `Avionics` — truth is never merged into the estimate the panels consume.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SimTruth {
    quat: Quat,
    pos_ned: [f32; 3],
    vel_ned: [f32; 3],
    valid_flags: u32,
    stamp: Stamp,
}

/// FC-owned arm/mode state under its own provenance stamp.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FcState {
    arm_state: u32,
    stamp: Stamp,
}

/// A telemetry sample. `pose`/`velocity` are absent when the host supplies no
/// coherent projection; the flattened `xM`/`yM`/`headingRad`/`linearXMps`/
/// `angularRadS` mirror them so a consumer can read either form.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TelemetryMessage {
    vehicle_id: u64,
    tick: u64,
    published_at_nanos: u64,
    pose: Option<Pose>,
    velocity: Option<Velocity>,
    x_m: Option<f32>,
    y_m: Option<f32>,
    heading_rad: Option<f32>,
    linear_x_mps: Option<f32>,
    angular_rad_s: Option<f32>,
    avionics: Option<Avionics>,
    sim_truth: Option<SimTruth>,
    fc_state: Option<FcState>,
}

/// Decodes one bare (non-length-delimited) datagram `Envelope`, returning
/// `{ kind, message }`. `kind` is `"TelemetrySample"`, `"Pong"`,
/// `"FrameRejected"`, or `"unknown"` (an undecodable buffer or an arm the
/// datagram channel does not carry). This is a drop-in for the browser's
/// `decodeBareEnvelope`.
#[wasm_bindgen(js_name = decodeDatagramEnvelope)]
#[must_use]
pub fn decode_datagram_envelope(bytes: &[u8]) -> JsValue {
    let Ok(envelope) = wire::Envelope::decode(bytes) else {
        return to_js(&Decoded {
            kind: "unknown",
            message: Empty {},
        });
    };
    match envelope.payload {
        Some(wire::envelope::Payload::TelemetrySample(sample)) => to_js(&Decoded {
            kind: "TelemetrySample",
            message: telemetry_message(sample),
        }),
        Some(wire::envelope::Payload::Pong(_)) => to_js(&Decoded {
            kind: "Pong",
            message: Empty {},
        }),
        Some(wire::envelope::Payload::FrameRejected(rejection)) => to_js(&Decoded {
            kind: "FrameRejected",
            message: FrameRejectedMessage {
                reason: rejection.reason,
            },
        }),
        _ => to_js(&Decoded {
            kind: "unknown",
            message: Empty {},
        }),
    }
}

fn telemetry_message(sample: wire::TelemetrySample) -> TelemetryMessage {
    let pose = sample.pose.map(|pose| Pose {
        x_m: pose.x_m,
        y_m: pose.y_m,
        heading_rad: pose.heading_rad,
    });
    let velocity = sample.velocity.map(|velocity| Velocity {
        linear_x_mps: velocity.linear_x_mps,
        linear_y_mps: velocity.linear_y_mps,
        angular_rad_s: velocity.angular_rad_s,
    });
    let (x_m, y_m, heading_rad) = match &pose {
        Some(pose) => (Some(pose.x_m), Some(pose.y_m), Some(pose.heading_rad)),
        None => (None, None, None),
    };
    let (linear_x_mps, angular_rad_s) = match &velocity {
        Some(velocity) => (Some(velocity.linear_x_mps), Some(velocity.angular_rad_s)),
        None => (None, None),
    };
    TelemetryMessage {
        vehicle_id: sample.vehicle.map_or(0, |vehicle| vehicle.value),
        tick: sample.tick.map_or(0, |tick| tick.value),
        published_at_nanos: sample.observed_at.map_or(0, |observed| observed.nanos),
        pose,
        velocity,
        x_m,
        y_m,
        heading_rad,
        linear_x_mps,
        angular_rad_s,
        avionics: sample.avionics.map(avionics_message),
        sim_truth: sample.sim_truth.and_then(|truth| sim_truth_message(*truth)),
        fc_state: sample.fc_state.and_then(|state| fc_state_message(*state)),
    }
}

/// `None` when the wire sample carries no provenance stamp: truth without
/// identity is unconsumable and is dropped, never defaulted.
fn sim_truth_message(state: wire::SimTruthState) -> Option<SimTruth> {
    let stamp = state.stamp?;
    // Exact-role gate: a truth lane whose stamp does not carry the
    // simulation-truth role is mislabeled and unconsumable.
    if stamp.role != wire::SourceRole::SimulationTruth as i32 {
        return None;
    }
    Some(SimTruth {
        quat: Quat {
            w: state.quat_w,
            x: state.quat_x,
            y: state.quat_y,
            z: state.quat_z,
        },
        pos_ned: [state.pos_n_m, state.pos_e_m, state.pos_d_m],
        vel_ned: [state.vel_n_mps, state.vel_e_mps, state.vel_d_mps],
        valid_flags: state.valid_flags,
        stamp: stamp_message(stamp),
    })
}

/// `None` when the wire report carries no provenance stamp: an unstamped
/// arm state is exactly what this lane exists to prevent.
fn fc_state_message(state: wire::FcState) -> Option<FcState> {
    let stamp = state.stamp?;
    // Exact-role gate: FC state must carry the FC-state role.
    if stamp.role != wire::SourceRole::FcState as i32 {
        return None;
    }
    Some(FcState {
        arm_state: state.arm_state,
        stamp: stamp_message(stamp),
    })
}

// Surfaces the deprecated wire lane `arm_state` verbatim (hosts leave it 0);
// consumers take arm from the stamped `fcState` message instead.
#[allow(deprecated)]
fn avionics_message(state: wire::AvionicsState) -> Avionics {
    let attitude_stamp = state.attitude_stamp.map(stamp_message);
    let kinematics_stamp = state.kinematics_stamp.map(stamp_message);
    let estimator_status_stamp = state.estimator_status_stamp.map(stamp_message);
    // A group's values are meaningful only when its acquisition stamp is
    // present; absent that, the group is null, never proto3 zero displayed as a
    // measurement (ADR-0018).
    let attitude = attitude_stamp.as_ref().map(|_| Attitude {
        quat: Quat {
            w: state.quat_w,
            x: state.quat_x,
            y: state.quat_y,
            z: state.quat_z,
        },
        rates: [state.rate_p_rad_s, state.rate_q_rad_s, state.rate_r_rad_s],
    });
    let kinematics = kinematics_stamp.as_ref().map(|_| Kinematics {
        pos_ned: [state.pos_n_m, state.pos_e_m, state.pos_d_m],
        vel_ned: [state.vel_n_mps, state.vel_e_mps, state.vel_d_mps],
    });
    Avionics {
        quat: attitude.map(|attitude| attitude.quat),
        rates: attitude.map(|attitude| attitude.rates),
        pos_ned: kinematics.map(|kinematics| kinematics.pos_ned),
        vel_ned: kinematics.map(|kinematics| kinematics.vel_ned),
        attitude,
        kinematics,
        valid_flags: state.valid_flags,
        quality: state.quality,
        arm_state: state.arm_state,
        attitude_stamp,
        kinematics_stamp,
        estimator_status_stamp,
    }
}

fn stamp_message(stamp: wire::MeasurementStamp) -> Stamp {
    Stamp {
        source_id: stamp.source_id,
        source_incarnation: (stamp.source_incarnation.len() == 16)
            .then(|| incarnation_hex(&stamp.source_incarnation)),
        source_epoch: stamp.source_epoch,
        sequence: stamp.sequence,
        acquired_at_nanos: stamp.acquired_at_ns,
        clock: stamp.clock,
        role: stamp.role,
        integrity: stamp.integrity,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{avionics_message, stamp_message, telemetry_message};
    use pilotage_protocol::wire;

    fn stamp(source_id: u64, incarnation: Vec<u8>) -> wire::MeasurementStamp {
        wire::MeasurementStamp {
            role: wire::SourceRole::OperationalEstimate as i32,
            integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
            source_id,
            source_epoch: 3,
            sequence: 9,
            acquired_at_ns: 123,
            clock: wire::MeasurementClock::Simulation as i32,
            source_incarnation: incarnation,
        }
    }

    #[allow(deprecated)]
    fn full_avionics() -> wire::AvionicsState {
        wire::AvionicsState {
            quat_w: 1.0,
            quat_x: 0.2,
            quat_y: 0.3,
            quat_z: 0.4,
            rate_p_rad_s: 0.5,
            rate_q_rad_s: 0.6,
            rate_r_rad_s: 0.7,
            pos_n_m: 10.0,
            pos_e_m: 11.0,
            pos_d_m: -2.0,
            vel_n_mps: 1.0,
            vel_e_mps: 2.0,
            vel_d_mps: 3.0,
            valid_flags: 0x0f,
            quality: 0,
            arm_state: 1,
            attitude_stamp: Some(stamp(7, vec![0xAB; 16])),
            kinematics_stamp: Some(stamp(8, vec![0xCD; 16])),
            estimator_status_stamp: Some(stamp(9, vec![0xEF; 16])),
        }
    }

    #[test]
    fn present_groups_flatten_and_mirror_their_nested_form() {
        let avionics = avionics_message(full_avionics());
        let attitude = avionics.attitude.expect("attitude present with its stamp");
        let quat = avionics.quat.expect("flat quat mirrors attitude");
        assert_eq!(quat.w, attitude.quat.w);
        assert_eq!(avionics.rates.expect("flat rates"), attitude.rates);
        let kinematics = avionics
            .kinematics
            .expect("kinematics present with its stamp");
        assert_eq!(avionics.pos_ned.expect("flat pos"), kinematics.pos_ned);
        assert_eq!(avionics.vel_ned.expect("flat vel"), kinematics.vel_ned);
    }

    #[test]
    fn absent_stamp_zeroes_no_group() {
        let mut state = full_avionics();
        state.attitude_stamp = None;
        let avionics = avionics_message(state);
        // The proto3 quat defaults are not a measurement without an attitude
        // stamp: the group and its flattened mirror must be absent, not zero.
        assert!(avionics.attitude.is_none());
        assert!(avionics.quat.is_none());
        assert!(avionics.rates.is_none());
        // The kinematics group, whose stamp survives, is unaffected.
        assert!(avionics.kinematics.is_some());
    }

    #[test]
    fn incarnation_is_hex_only_when_sixteen_bytes() {
        assert_eq!(
            stamp_message(stamp(1, vec![0xAB; 16]))
                .source_incarnation
                .as_deref(),
            Some("abababababababababababababababab")
        );
        assert!(
            stamp_message(stamp(1, vec![0xAB; 4]))
                .source_incarnation
                .is_none()
        );
        assert!(
            stamp_message(stamp(1, Vec::new()))
                .source_incarnation
                .is_none()
        );
    }

    #[test]
    fn pose_absence_leaves_flattened_fields_absent() {
        let sample = wire::TelemetrySample {
            vehicle: Some(wire::VehicleId { value: 1 }),
            tick: Some(wire::SimTick { value: 5 }),
            observed_at: Some(wire::MonoTimestamp { nanos: 900 }),
            pose: None,
            velocity: Some(wire::Velocity2d {
                linear_x_mps: 4.0,
                linear_y_mps: 0.0,
                angular_rad_s: 0.1,
            }),
            avionics: None,
            sim_truth: None,
            fc_state: None,
            gimbal: None,
        };
        let message = telemetry_message(sample);
        assert_eq!(message.vehicle_id, 1);
        assert!(message.pose.is_none());
        assert!(message.x_m.is_none());
        // Velocity present flattens through.
        assert_eq!(message.linear_x_mps, Some(4.0));
        assert_eq!(message.angular_rad_s, Some(0.1));
        assert!(message.avionics.is_none());
    }
}
