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

mod groups;

use groups::{
    Avionics, FcState, Gimbal, NavGuidance, SimTruth, avionics_message, fc_state_message,
    gimbal_message, nav_guidance_message, sim_truth_message,
};

/// `{ kind, message }`, the browser's envelope-decode result shape.
#[derive(Serialize)]
struct Decoded<M> {
    kind: &'static str,
    message: M,
}

/// The `Pong` and `unknown` arms carry no fields the viewer reads.
#[derive(Serialize)]
struct Empty {}

/// A `FrameRejected` with its full addressing, so the viewer can attribute
/// the rejection to a scope and react to a fenced-out hold (host watchdog
/// revocation) instead of streaming rejected frames blind.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrameRejectedMessage {
    reason: i32,
    scope: String,
    sequence: u32,
    current_generation: u64,
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

/// A telemetry sample. `pose`/`velocity` are absent when the host supplies no
/// coherent projection; the flattened `xM`/`yM`/`headingRad`/`linearXMps`/
/// `angularRadS` mirror them so a consumer can read either form.
///
/// Every stamped sub-message the wire carries has a field here. A group the
/// host encodes and this struct omits is invisible to the viewer without any
/// decode error to notice it by, so the wasm/JS conformance test pins the
/// presence of each one against the JS reference decoder.
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
    gimbal: Option<Gimbal>,
    nav_guidance: Option<NavGuidance>,
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
                scope: rejection.scope.map(|scope| scope.value).unwrap_or_default(),
                sequence: rejection.sequence.map_or(0, |sequence| sequence.value),
                current_generation: rejection
                    .current_generation
                    .map_or(0, |generation| generation.value),
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
        gimbal: sample.gimbal.and_then(|gimbal| gimbal_message(*gimbal)),
        nav_guidance: sample
            .nav_guidance
            .and_then(|guidance| nav_guidance_message(*guidance)),
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
mod tests;
