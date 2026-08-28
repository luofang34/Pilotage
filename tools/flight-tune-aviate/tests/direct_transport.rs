//! The direct transport driving the real Aviate command sender.
//!
//! The transport owns no socket, so its unit tests drive a recording
//! sender. This suite closes that gap: it binds the transport's command
//! port to the real [`FlightUplink`], reads the MAVLink datagram that
//! reaches a fake flight controller, and reports the decoded wire values
//! back as the effective setpoint. What the transport verifies is
//! therefore what actually left the process.
//!
//! SIM / NOT FOR FLIGHT.

#![allow(clippy::expect_used, clippy::panic)]

use std::net::UdpSocket;
use std::time::Duration;

use flight_tune::{
    ArtifactIdentity, ControlChannel, ControlFamily, Digest, ExecutionTarget, PhysicalUnit,
    ReferenceRule, SimulatorCapability, SimulatorSessionReceipt, StimulusEnvelope, StimulusMapping,
    VehicleBindingReceipt,
};
use flight_tune_aviate::direct_transport::{
    CausalReadbackBound, DirectBaselineRequest, DirectCommandSender, DirectEnactment,
    DirectSenderError, DirectSenderIdentity, DirectSetpoint, DirectStepRequest, DirectTransport,
    DirectTransportRequest, EffectiveSetpointReport, TransmittedDirectCommand,
    direct_transport_identity,
};
use pilotage_adapter_aviate::{
    AviateProfile, ExactDirectSetpoint, FlightUplink, SimulatorDirectAuthority,
};
use sha2::{Digest as _, Sha256};

/// One simulator sample at the flight controller's setpoint rate.
const SAMPLE_PERIOD_NS: u64 = 12_500_000;
/// The identified hover trim of the test airframe.
const HOVER_TRIM: f64 = 0.72;
/// A single-precision MAVLink attitude frame round-trips through a
/// quaternion, so the declared tolerance is a float tolerance and not zero.
const TOLERANCE: f64 = 1e-5;

/// The transport's command port over the real Aviate uplink.
///
/// The effective setpoint is decoded from the datagram the fake flight
/// controller received, so a shaped or clamped command could not report
/// itself as the requested target.
struct AviateDirectSender {
    uplink: FlightUplink,
    authority: SimulatorDirectAuthority,
    flight_controller: UdpSocket,
    now_ns: u64,
    effective: Option<EffectiveSetpointReport>,
}

impl AviateDirectSender {
    fn bind() -> Self {
        let flight_controller = UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
        flight_controller
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout");
        let authority = SimulatorDirectAuthority::for_profile(AviateProfile::Simulation)
            .expect("simulation authority");
        let mut uplink = FlightUplink::bind_to(flight_controller.local_addr().expect("FC address"))
            .expect("uplink");
        uplink.open_setpoint_stream(&authority);
        Self {
            uplink,
            authority,
            flight_controller,
            now_ns: 0,
            effective: None,
        }
    }

    fn receive(&self) -> Vec<u8> {
        let mut buffer = [0_u8; 128];
        let (len, _) = self
            .flight_controller
            .recv_from(&mut buffer)
            .expect("a frame reached the FC");
        buffer[..len].to_vec()
    }

    fn expect_silence(&self) {
        let mut buffer = [0_u8; 128];
        self.flight_controller
            .set_read_timeout(Some(Duration::from_millis(50)))
            .expect("read timeout");
        let received = self.flight_controller.recv_from(&mut buffer);
        self.flight_controller
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout");
        assert!(received.is_err(), "no datagram may have left the process");
    }
}

fn field(frame: &[u8], offset: usize) -> f32 {
    let start = 10 + offset;
    let bytes = frame.get(start..start + 4).expect("payload field");
    f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Recovers one direct setpoint from a received SET_ATTITUDE_TARGET frame.
fn decode_setpoint(frame: &[u8]) -> DirectSetpoint {
    assert_eq!(frame[7], 82, "SET_ATTITUDE_TARGET id");
    let (qw, qx, qy, qz) = (
        field(frame, 4),
        field(frame, 8),
        field(frame, 12),
        field(frame, 16),
    );
    DirectSetpoint {
        roll_rad: f64::from((2.0 * (qw * qx + qy * qz)).atan2(1.0 - 2.0 * (qx * qx + qy * qy))),
        pitch_rad: f64::from((2.0 * (qw * qy - qz * qx)).asin()),
        yaw_rad: f64::from((2.0 * (qw * qz + qx * qy)).atan2(1.0 - 2.0 * (qy * qy + qz * qz))),
        collective_force: f64::from(field(frame, 32)),
    }
}

impl DirectCommandSender for AviateDirectSender {
    fn command_endpoint(&self) -> String {
        self.uplink.command_endpoint().to_string()
    }

    fn now_ns(&mut self) -> Result<u64, DirectSenderError> {
        Ok(self.now_ns)
    }

    fn transmit_exact_blocking(
        &mut self,
        setpoint: DirectSetpoint,
    ) -> Result<TransmittedDirectCommand, DirectSenderError> {
        let transmitted = self
            .uplink
            .send_exact_direct_setpoint(
                &self.authority,
                ExactDirectSetpoint {
                    roll_rad: setpoint.roll_rad as f32,
                    pitch_rad: setpoint.pitch_rad as f32,
                    yaw_rad: setpoint.yaw_rad as f32,
                    collective_force: setpoint.collective_force as f32,
                },
            )
            .map_err(|error| DirectSenderError::new("transmit", error.to_string()))?;
        let wire = decode_setpoint(&self.receive());
        let transmitted_at_ns = self.now_ns;
        let sample_time_ns = transmitted_at_ns + SAMPLE_PERIOD_NS;
        self.now_ns = sample_time_ns;
        self.effective = Some(EffectiveSetpointReport {
            setpoint: wire,
            sample_sequence: sample_time_ns / SAMPLE_PERIOD_NS,
            sample_time_ns,
            estimate_time_ns: sample_time_ns,
            simulator_truth_time_ns: sample_time_ns,
        });
        Ok(TransmittedDirectCommand {
            setpoint: wire,
            sender: DirectSenderIdentity {
                endpoint: self.command_endpoint(),
                system_id: self.uplink.expected_source().0,
                component_id: self.uplink.expected_source().1,
                sequence: transmitted.sequence,
                time_boot_ms: transmitted.time_boot_ms,
                frame_digest: Digest::from_bytes(Sha256::digest(transmitted.frame).into()),
            },
            transmitted_at_ns,
        })
    }

    fn effective_setpoint_blocking(
        &mut self,
    ) -> Result<Option<EffectiveSetpointReport>, DirectSenderError> {
        Ok(self.effective)
    }

    fn is_stable_blocking(&mut self) -> Result<bool, DirectSenderError> {
        Ok(true)
    }
}

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

fn authorize(sender: &AviateDirectSender, target: ExecutionTarget) -> Option<DirectTransport> {
    let capability = SimulatorCapability::for_test(session_receipt());
    let simulator = session_receipt();
    let vehicle = vehicle_receipt();
    let transport: ArtifactIdentity = direct_transport_identity().expect("transport identity");
    DirectTransport::authorize(
        &DirectTransportRequest {
            capability: &capability,
            simulator: &simulator,
            vehicle: &vehicle,
            target,
            transport: &transport,
            readback: CausalReadbackBound::new(SAMPLE_PERIOD_NS, SAMPLE_PERIOD_NS)
                .expect("readback bound"),
            tolerance: TOLERANCE,
        },
        sender,
    )
    .ok()
}

fn baseline_request() -> DirectBaselineRequest {
    DirectBaselineRequest {
        measured_roll_rad: 0.0,
        measured_pitch_rad: 0.0,
        measured_yaw_rad: 0.0,
        hover_trim: HOVER_TRIM,
        run_intent_digest: digest(9),
        max_commands: 4,
    }
}

fn tilt_envelope() -> StimulusEnvelope {
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

fn roll_step(normalized: f64) -> DirectStepRequest {
    DirectStepRequest {
        family: ControlFamily::DirectAttitudeThrust,
        channel: ControlChannel::Roll,
        mapping: StimulusMapping::AffineExact,
        envelope: tilt_envelope(),
        normalized,
    }
}

#[test]
fn the_transport_drives_the_real_command_sender_to_an_exact_target() {
    let mut sender = AviateDirectSender::bind();
    let mut transport = authorize(&sender, ExecutionTarget::Simulator).expect("authorized");
    let baseline = transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    assert_eq!(baseline.setpoint().collective_force, HOVER_TRIM);

    let prepared = transport.prepare_step(&roll_step(1.0)).expect("prepared");
    let outcome = transport
        .enact_blocking(&mut sender, &prepared)
        .expect("enacted step");

    let DirectEnactment::Enacted(record) = outcome else {
        panic!("expected an enacted command, got {outcome:?}");
    };
    assert_eq!(
        prepared.requested().roll_rad,
        0.25,
        "the envelope resolves the full-scale roll target"
    );
    // The effective setpoint here IS the decoded wire value, so this is a
    // statement about the datagram and not about the transport's own copy.
    assert!(
        (record.effective.roll_rad - 0.25).abs() < TOLERANCE,
        "the wire carried {} rad",
        record.effective.roll_rad
    );
    assert!((record.effective.pitch_rad).abs() < TOLERANCE);
    assert!((record.effective.collective_force - HOVER_TRIM).abs() < TOLERANCE);
    assert_eq!(
        record.times.effective_at_ns,
        record.times.transmitted_at_ns + SAMPLE_PERIOD_NS,
        "the readback is the exact next simulator sample"
    );
    assert_eq!(record.sender.endpoint, sender.command_endpoint());
    assert_eq!(record.channel, ControlChannel::Roll);
    assert_eq!(record.family, ControlFamily::DirectAttitudeThrust);
    assert_ne!(record.sender.frame_digest, Digest::from_bytes([0; 32]));
}

#[test]
fn a_hardware_target_cannot_bind_the_real_command_sender() {
    let sender = AviateDirectSender::bind();

    assert!(
        authorize(&sender, ExecutionTarget::RealVehicle).is_none(),
        "a real-vehicle target must not reach the exact direct path"
    );
    sender.expect_silence();
}

#[test]
fn a_changed_prepared_target_never_reaches_the_command_link() {
    let mut sender = AviateDirectSender::bind();
    let mut transport = authorize(&sender, ExecutionTarget::Simulator).expect("authorized");
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    let tampered = transport
        .prepare_step(&roll_step(1.0))
        .expect("prepared")
        .with_requested_for_test(DirectSetpoint {
            roll_rad: 0.5,
            pitch_rad: 0.0,
            yaw_rad: 0.0,
            collective_force: HOVER_TRIM,
        });

    let result = transport.enact_blocking(&mut sender, &tampered);

    assert!(result.is_err(), "a changed prepared target must be refused");
    sender.expect_silence();
}

#[test]
fn the_operator_family_still_reaches_the_flight_controller_after_a_direct_step() {
    // The operator law keeps its own message and its own path. The exact
    // direct step must leave that path working; the adapter suite proves
    // the shaped numbers themselves are untouched, on a controlled clock.
    let mut sender = AviateDirectSender::bind();
    let mut transport = authorize(&sender, ExecutionTarget::Simulator).expect("authorized");
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    let prepared = transport.prepare_step(&roll_step(1.0)).expect("prepared");
    transport
        .enact_blocking(&mut sender, &prepared)
        .expect("enacted step");

    sender
        .uplink
        .send_stick_frame(0.0, 1.0, 0.6, 0.5, 0.0, [0.0; 3], Some([0.0; 3]), None);
    let frame = sender.receive();

    assert_eq!(
        frame[7], 84,
        "the operator family still sends SET_POSITION_TARGET_LOCAL_NED"
    );
    assert_eq!(
        sender.uplink.send_failures(),
        0,
        "no command was refused by the link"
    );
}

#[test]
fn the_transport_refuses_a_stimulus_the_real_sender_could_not_carry_exactly() {
    let mut sender = AviateDirectSender::bind();
    let mut transport = authorize(&sender, ExecutionTarget::Simulator).expect("authorized");
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    // The uplink's direct tilt envelope stops well short of this target, so
    // a shaped path would clamp it and report the clamped value as the
    // command. The exact path has no such outcome.
    let mut beyond_envelope = roll_step(1.0);
    beyond_envelope.envelope.positive_endpoint = 1.4;

    let prepared = transport
        .prepare_step(&beyond_envelope)
        .expect("prepared step");
    let result = transport.enact_blocking(&mut sender, &prepared);

    assert!(
        result.is_err(),
        "a target the sender cannot carry exactly must fail, not clamp"
    );
    sender.expect_silence();
}

#[test]
fn the_operator_velocity_family_cannot_reach_the_real_command_sender() {
    let mut sender = AviateDirectSender::bind();
    let mut transport = authorize(&sender, ExecutionTarget::Simulator).expect("authorized");
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    let mut operator = roll_step(1.0);
    operator.family = ControlFamily::OperatorVelocity;

    assert!(transport.prepare_step(&operator).is_err());
    assert!(transport.prepare_release(&operator).is_err());
    sender.expect_silence();
}
