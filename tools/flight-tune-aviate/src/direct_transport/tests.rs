//! Conformance tests for the simulator-only direct transport.

#![allow(clippy::expect_used, clippy::panic)]

mod authority;
mod baseline;
mod readback;
mod revoke;
mod sender;
mod step;

use flight_tune::{
    ArtifactIdentity, ControlChannel, ControlFamily, Digest, ExecutionTarget, PhysicalUnit,
    ReferenceRule, SimulatorCapability, SimulatorSessionReceipt, StimulusEnvelope, StimulusMapping,
    VehicleBindingReceipt,
};

use super::{
    CausalReadbackBound, DirectBaselineRequest, DirectStepRequest, DirectTransport,
    DirectTransportError, DirectTransportRequest, direct_transport_identity,
};
use sender::RecordingSender;

/// One simulator sample at the flight controller's 80 Hz setpoint rate.
const SAMPLE_PERIOD_NS: u64 = 12_500_000;
/// The identified hover trim of the test airframe.
const HOVER_TRIM: f64 = 0.72;
/// The declared numeric tolerance for a target comparison.
const TOLERANCE: f64 = 1e-9;

fn digest(seed: u8) -> Digest {
    Digest::from_bytes([seed; 32])
}

fn session_receipt() -> SimulatorSessionReceipt {
    SimulatorSessionReceipt {
        session_digest: digest(1),
        simulator_digest: digest(2),
        airframe_digest: digest(3),
    }
}

fn vehicle_receipt() -> VehicleBindingReceipt {
    VehicleBindingReceipt {
        session_digest: digest(1),
        vehicle_digest: digest(4),
        scenario_runtime_digest: digest(5),
    }
}

fn capability() -> SimulatorCapability {
    SimulatorCapability::for_test(session_receipt())
}

fn transport_identity() -> ArtifactIdentity {
    direct_transport_identity().expect("transport identity")
}

fn readback_bound() -> CausalReadbackBound {
    CausalReadbackBound::new(SAMPLE_PERIOD_NS, SAMPLE_PERIOD_NS).expect("readback bound")
}

fn authorize(sender: &RecordingSender) -> DirectTransport {
    authorize_with(
        sender,
        ExecutionTarget::Simulator,
        &session_receipt(),
        &vehicle_receipt(),
    )
    .expect("authorized transport")
}

/// A transport that accepts only a sample at exactly the query time.
fn authorize_without_skew(sender: &RecordingSender) -> DirectTransport {
    let bound = CausalReadbackBound::new(SAMPLE_PERIOD_NS, 0).expect("readback bound");
    authorize_bound(
        sender,
        ExecutionTarget::Simulator,
        &session_receipt(),
        &vehicle_receipt(),
        bound,
    )
    .expect("authorized transport")
}

fn authorize_with(
    sender: &RecordingSender,
    target: ExecutionTarget,
    simulator: &SimulatorSessionReceipt,
    vehicle: &VehicleBindingReceipt,
) -> Result<DirectTransport, DirectTransportError> {
    authorize_bound(sender, target, simulator, vehicle, readback_bound())
}

fn authorize_bound(
    sender: &RecordingSender,
    target: ExecutionTarget,
    simulator: &SimulatorSessionReceipt,
    vehicle: &VehicleBindingReceipt,
    readback: CausalReadbackBound,
) -> Result<DirectTransport, DirectTransportError> {
    let capability = capability();
    let transport = transport_identity();
    DirectTransport::authorize(
        &DirectTransportRequest {
            capability: &capability,
            simulator,
            vehicle,
            target,
            transport: &transport,
            readback,
            tolerance: TOLERANCE,
        },
        sender,
    )
}

fn baseline_request() -> DirectBaselineRequest {
    DirectBaselineRequest {
        measured_roll_rad: 0.01,
        measured_pitch_rad: -0.02,
        measured_yaw_rad: 1.2,
        hover_trim: HOVER_TRIM,
        run_intent_digest: digest(9),
        max_commands: 8,
    }
}

/// An envelope whose neutral is zero, so a normalized zero resolves to the
/// frozen baseline itself.
fn attitude_envelope() -> StimulusEnvelope {
    StimulusEnvelope {
        id: "alia-tilt-v1".to_owned(),
        revision: 1,
        unit: PhysicalUnit::Radians,
        reference: ReferenceRule::EffectiveSetpointAtEntry,
        negative_endpoint: -0.25,
        neutral: 0.0,
        positive_endpoint: 0.25,
    }
}

fn collective_envelope() -> StimulusEnvelope {
    StimulusEnvelope {
        id: "alia-collective-v1".to_owned(),
        revision: 1,
        unit: PhysicalUnit::NormalizedCollectiveForce,
        reference: ReferenceRule::IdentifiedHoverTrim,
        negative_endpoint: -0.1,
        neutral: 0.0,
        positive_endpoint: 0.1,
    }
}

fn step_request(channel: ControlChannel, normalized: f64) -> DirectStepRequest {
    let envelope = match channel {
        ControlChannel::Vertical => collective_envelope(),
        ControlChannel::Roll | ControlChannel::Pitch | ControlChannel::Yaw => attitude_envelope(),
    };
    DirectStepRequest {
        family: ControlFamily::DirectAttitudeThrust,
        channel,
        mapping: StimulusMapping::AffineExact,
        envelope,
        normalized,
    }
}

/// A transport with a frozen baseline, and the sender it froze against.
fn frozen() -> (DirectTransport, RecordingSender) {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    sender.clear_transmitted();
    (transport, sender)
}
