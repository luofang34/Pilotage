#![allow(clippy::expect_used, clippy::panic)]

//! Engine tests. The wire fixtures are the SAME files the browser suites
//! execute (`clients/web-control/*.json`), so the native and web paths
//! cannot drift: a divergence reddens exactly one of them.

use pilotage_protocol::wire;
use prost::Message;

use crate::{
    ClientAction, ClientConfig, ClientEngine, ClientPhase, ControlCommand, ControlLane,
    ModuleEvent, ReconnectPolicy, StreamId, TransportEvent,
};

const WELCOME_FIXTURE: &str =
    include_str!("../../../clients/web-control/server-welcome-fixture.json");
const TYPED_FRAME_FIXTURE: &str =
    include_str!("../../../clients/web-control/typed-frame-fixture.json");

fn engine() -> ClientEngine {
    ClientEngine::new(ClientConfig {
        client_name: "test-client".into(),
        reconnect: ReconnectPolicy::default(),
    })
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("fixture hex digits"))
        .collect()
}

fn fixture_hex(doc: &str, key: &str) -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_str(doc).expect("fixture parses");
    hex_bytes(value[key].as_str().expect("fixture key present"))
}

/// Admits the engine with a minimal host-shaped welcome and returns the
/// emitted actions.
fn admit(engine: &mut ClientEngine, session: u64, principal: u64) -> Vec<ClientAction> {
    let welcome = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::ServerWelcome(
            wire::ServerWelcome {
                session: Some(wire::SessionId { value: session }),
                principal: Some(wire::PrincipalId { value: principal }),
                host_capabilities: Some(wire::HostCapabilities {
                    host_version: "test-host".into(),
                    vehicles: vec![wire::VehicleDescriptor {
                        vehicle: Some(wire::VehicleId { value: 1 }),
                        display_name: "vehicle".into(),
                        scopes: vec![wire::ScopeDescriptor {
                            scope: Some(wire::ScopeId {
                                value: "vehicle.motion".into(),
                            }),
                            ..Default::default()
                        }],
                        supported_modes: Vec::new(),
                    }],
                    supported_modes: Vec::new(),
                }),
                scope_holders: Vec::new(),
            },
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&welcome);
    engine.handle(TransportEvent::Connected, 0);
    engine.handle(TransportEvent::BootstrapReceived(bytes), 0)
}

/// Grants the pending lease at `generation` through the bootstrap stream.
fn grant(engine: &mut ClientEngine, generation: u64) -> Vec<ClientAction> {
    let response = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::LeaseResponse(
            wire::LeaseResponse {
                vehicle: Some(wire::VehicleId { value: 1 }),
                scope: Some(wire::ScopeId {
                    value: "vehicle.motion".into(),
                }),
                granted: true,
                generation: Some(wire::Generation { value: generation }),
                reason: 0,
            },
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&response);
    engine.handle(TransportEvent::BootstrapReceived(bytes), 0)
}

#[test]
fn the_shared_welcome_fixture_admits_with_the_negotiated_capabilities() {
    // The exact bytes the real host emitted and the browser decoder reads.
    let envelope_bytes = fixture_hex(WELCOME_FIXTURE, "envelopeHex");
    let mut engine = engine();
    engine.handle(TransportEvent::Connected, 0);
    let actions = engine.handle(TransportEvent::BootstrapReceived(envelope_bytes), 0);

    let admitted = actions.iter().find_map(|action| match action {
        ClientAction::Emit(ModuleEvent::Admitted(admission)) => Some(admission),
        _ => None,
    });
    let admission = admitted.expect("the fixture welcome admits");
    assert_eq!(admission.session_id, 0);
    assert_eq!(admission.host_version, "welcome-fixture");
    let scopes: Vec<&str> = admission.vehicles[0]
        .scopes
        .iter()
        .map(|s| s.scope.as_str())
        .collect();
    assert_eq!(scopes, ["vehicle.motion", "vehicle.motion.direct"]);
    let motion = &admission.vehicles[0].scopes[0];
    let velocity = &motion.intents[0];
    assert_eq!(velocity.family, wire::IntentFamily::Velocity as i32);
    assert_eq!(velocity.frames.len(), 2);
    assert!((velocity.max_linear - 3.0).abs() < f32::EPSILON);
    assert!((velocity.max_vertical - 1.5).abs() < f32::EPSILON);
    assert!((velocity.max_angular - 0.9).abs() < f32::EPSILON);
    assert!(admission.offers_control());
}

#[test]
fn the_shared_typed_frame_fixture_is_reproduced_byte_for_byte() {
    // The browser chain produced these bytes; the native lane must produce
    // the same ones from the same fencing facts.
    let expected = fixture_hex(TYPED_FRAME_FIXTURE, "envelopeHex");
    let fixture: serde_json::Value =
        serde_json::from_str(TYPED_FRAME_FIXTURE).expect("fixture parses");
    let e = &fixture["expected"];

    let mut lane = ControlLane::new(
        e["sessionId"].as_u64().expect("session"),
        e["vehicleId"].as_u64().expect("vehicle"),
        e["scope"].as_str().expect("scope").to_owned(),
        e["generation"].as_u64().expect("generation"),
    );
    lane.bind_profile(
        u32::try_from(e["profileRevision"].as_u64().expect("profile")).expect("u32"),
        u32::try_from(e["activationRevision"].as_u64().expect("activation")).expect("u32"),
    );
    let sequence = u32::try_from(e["sequence"].as_u64().expect("sequence")).expect("u32");
    lane.restore_counters(sequence.wrapping_sub(1), 0);

    let velocity = &e["velocity"];
    #[allow(clippy::cast_possible_truncation)]
    let intent = wire::ControlIntent {
        family: Some(wire::control_intent::Family::Velocity(
            wire::VelocityIntent {
                frame: i32::try_from(velocity["frame"].as_i64().expect("frame")).expect("i32"),
                vx: velocity["vx"].as_f64().expect("vx") as f32,
                vy: velocity["vy"].as_f64().expect("vy") as f32,
                vz: velocity["vz"].as_f64().expect("vz") as f32,
                yaw_rate: velocity["yawRate"].as_f64().expect("yawRate") as f32,
            },
        )),
    };
    let bytes = lane.frame(
        ControlCommand::Intent(intent),
        e["sampledAtNanos"].as_u64().expect("sampledAt"),
    );
    assert_eq!(
        bytes, expected,
        "native frame bytes must equal the browser's"
    );
}

#[test]
fn a_lease_is_only_armed_for_the_request_the_shell_made() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    assert_eq!(*engine.phase(), ClientPhase::Admitted);
    assert!(!engine.holds_control());

    // A grant nobody asked for does not arm control.
    grant(&mut engine, 3);
    assert!(!engine.holds_control());

    let actions = engine.request_lease(1, "vehicle.motion");
    assert!(matches!(actions[0], ClientAction::SendBootstrap(_)));
    grant(&mut engine, 4);
    assert!(engine.holds_control());
}

#[test]
fn control_frames_carry_the_grant_fencing_and_advance_sequence() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");
    grant(&mut engine, 4);

    let mut frames = Vec::new();
    for _ in 0..2 {
        let actions = engine.control_frame(
            ControlCommand::Legacy(wire::ControlPayload {
                axes: vec![wire::AxisSample {
                    axis_id: 0,
                    value: 1.0,
                }],
                edges: Vec::new(),
            }),
            10,
        );
        let ClientAction::SendDatagram(bytes) = &actions[0] else {
            panic!("a held lease produces a datagram");
        };
        let envelope = wire::Envelope::decode(bytes.as_slice()).expect("frame decodes");
        let Some(wire::envelope::Payload::ControlFrame(frame)) = envelope.payload else {
            panic!("the datagram is a control frame");
        };
        frames.push(frame);
    }
    assert_eq!(frames[0].generation.as_ref().map(|g| g.value), Some(4));
    assert_eq!(frames[0].sequence.as_ref().map(|s| s.value), Some(1));
    assert_eq!(frames[1].sequence.as_ref().map(|s| s.value), Some(2));
    assert_eq!(frames[0].session.as_ref().map(|s| s.value), Some(7));
}

#[test]
fn without_a_lease_no_control_frame_leaves_the_engine() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    let actions = engine.control_frame(ControlCommand::Legacy(wire::ControlPayload::default()), 10);
    assert!(actions.is_empty(), "unfenced input must not be sendable");
}

#[test]
fn recovery_restores_observation_and_never_requests_a_lease() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");
    grant(&mut engine, 4);
    assert!(engine.holds_control());

    let actions = engine.handle(
        TransportEvent::TransportLost {
            detail: "loss".into(),
        },
        1_000,
    );
    assert!(!engine.holds_control(), "authority does not survive a loss");
    let retry_at = actions.iter().find_map(|action| match action {
        ClientAction::ScheduleReconnect { at_ms } => Some(*at_ms),
        _ => None,
    });
    assert_eq!(retry_at, Some(1_500), "first retry after the initial delay");

    // Recovery: connect again. Every emitted byte must be the hello, never
    // a lease request.
    let actions = engine.handle(TransportEvent::Connected, 1_500);
    for action in &actions {
        if let ClientAction::SendBootstrap(bytes) = action {
            let (envelope, _) = pilotage_protocol::decode_envelope_length_delimited(bytes)
                .expect("bootstrap bytes decode");
            assert!(
                matches!(
                    envelope.payload,
                    Some(wire::envelope::Payload::ClientHello(_))
                ),
                "recovery sends hello only"
            );
        }
    }
}

#[test]
fn backoff_doubles_and_is_bounded() {
    let mut engine = engine();
    let mut last = 0;
    let mut delays = Vec::new();
    for _ in 0..8 {
        let actions = engine.handle(
            TransportEvent::TransportLost {
                detail: "loss".into(),
            },
            last,
        );
        let at = actions
            .iter()
            .find_map(|action| match action {
                ClientAction::ScheduleReconnect { at_ms } => Some(*at_ms),
                _ => None,
            })
            .expect("a loss schedules a retry");
        delays.push(at - last);
        last = at;
    }
    assert_eq!(&delays[..3], &[500, 1_000, 2_000]);
    assert!(delays.iter().all(|d| *d <= 15_000));
    assert_eq!(*delays.last().expect("delays"), 15_000);
}

#[test]
fn the_authority_stream_is_tag_routed_and_unknown_tags_fail_closed() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);

    let event = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::AuthorityEvent(
            wire::AuthorityEvent {
                event: Some(wire::authority_event::Event::ScopeLeaseGranted(
                    wire::ScopeLeaseGranted {
                        principal: Some(wire::PrincipalId { value: 9 }),
                        vehicle: Some(wire::VehicleId { value: 1 }),
                        scope: Some(wire::ScopeId {
                            value: "vehicle.motion".into(),
                        }),
                        generation: Some(wire::Generation { value: 6 }),
                        reason: String::new(),
                        authority_class: 0,
                    },
                )),
            },
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&event);

    // The session-events stream leads with its kind tag; the payload may be
    // split across reads.
    engine.handle(TransportEvent::UniStreamOpened(StreamId(1)), 0);
    let mut tagged = vec![0x01];
    tagged.extend_from_slice(&bytes[..3]);
    let first = engine.handle(TransportEvent::UniStreamReceived(StreamId(1), tagged), 0);
    assert!(first.is_empty(), "a partial envelope emits nothing");
    let rest = engine.handle(
        TransportEvent::UniStreamReceived(StreamId(1), bytes[3..].to_vec()),
        0,
    );
    assert!(matches!(
        rest[0],
        ClientAction::Emit(ModuleEvent::Authority(_))
    ));
    assert_eq!(
        engine
            .authority()
            .holder(1, "vehicle.motion")
            .and_then(|h| h.holder_id),
        Some(9)
    );

    // A stream with an unknown tag is discarded without touching others.
    engine.handle(TransportEvent::UniStreamOpened(StreamId(2)), 0);
    let mut unknown = vec![0x7f];
    unknown.extend_from_slice(&bytes);
    let none = engine.handle(TransportEvent::UniStreamReceived(StreamId(2), unknown), 0);
    assert!(none.is_empty(), "an unknown stream kind emits nothing");
    let again = engine.handle(
        TransportEvent::UniStreamReceived(StreamId(1), {
            let mut b = Vec::new();
            b.extend_from_slice(&bytes);
            b
        }),
        0,
    );
    assert!(
        matches!(again[0], ClientAction::Emit(ModuleEvent::Authority(_))),
        "the session-events stream still works"
    );
}

#[test]
fn a_schema_the_client_does_not_speak_stops_the_engine() {
    let mut engine = engine();
    engine.handle(TransportEvent::Connected, 0);
    let alien = wire::Envelope {
        schema_version: 2,
        payload: Some(wire::envelope::Payload::ServerWelcome(
            wire::ServerWelcome::default(),
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&alien);
    let actions = engine.handle(TransportEvent::BootstrapReceived(bytes), 0);
    assert!(matches!(actions[0], ClientAction::Stop(_)));
    assert_eq!(*engine.phase(), ClientPhase::Stopped);

    // Stopped means stopped: a later loss schedules nothing.
    let after = engine.handle(
        TransportEvent::TransportLost {
            detail: "late".into(),
        },
        99,
    );
    assert!(after.is_empty());
}

#[test]
fn telemetry_and_rejections_arrive_as_typed_module_events() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);

    let telemetry = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::TelemetrySample(
            wire::TelemetrySample::default(),
        )),
    };
    let actions = engine.handle(
        TransportEvent::DatagramReceived(telemetry.encode_to_vec()),
        0,
    );
    assert!(matches!(
        actions[0],
        ClientAction::Emit(ModuleEvent::Telemetry(_))
    ));

    let rejected = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::FrameRejected(
            wire::FrameRejected::default(),
        )),
    };
    let actions = engine.handle(
        TransportEvent::DatagramReceived(rejected.encode_to_vec()),
        0,
    );
    assert!(matches!(
        actions[0],
        ClientAction::Emit(ModuleEvent::ControlRejected(_))
    ));
}

#[test]
fn a_full_climb_demand_scales_onto_the_fixture_envelope() {
    // The shared typed-frame fixture was produced by the browser from a
    // full climb demand against the 3.0/1.5/0.9 advertised envelope; the
    // native scaling must reach the same setpoint.
    let envelope_bytes = fixture_hex(WELCOME_FIXTURE, "envelopeHex");
    let mut engine = engine();
    engine.handle(TransportEvent::Connected, 0);
    engine.handle(TransportEvent::BootstrapReceived(envelope_bytes), 0);
    let admission = engine.admission().expect("fixture admits").clone();

    let capability = crate::intent_capability(
        &admission,
        1,
        "vehicle.motion",
        wire::IntentFamily::Velocity,
    );
    let intent = crate::velocity_intent(
        crate::MotionDemand {
            roll: 0.0,
            pitch: 0.0,
            throttle: 1.0,
            yaw: 0.0,
        },
        capability,
    )
    .expect("the advertised scope builds an intent");
    let Some(wire::control_intent::Family::Velocity(velocity)) = intent.family else {
        panic!("a velocity capability builds a velocity intent");
    };
    assert!(
        (velocity.vz - (-1.5)).abs() < f32::EPSILON,
        "full climb is -maxVertical"
    );
    assert!((velocity.vx).abs() < f32::EPSILON);

    // No advertisement, no intent: fail closed.
    assert!(
        crate::velocity_intent(
            crate::MotionDemand {
                roll: 0.0,
                pitch: 0.0,
                throttle: 1.0,
                yaw: 0.0
            },
            None
        )
        .is_none()
    );
}

#[test]
fn the_first_grant_announces_the_profile_and_binds_the_lane() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");
    let actions = grant(&mut engine, 4);

    // The grant's actions carry exactly one activation announcement.
    let activation = actions
        .iter()
        .find_map(|action| match action {
            ClientAction::SendBootstrap(bytes) => {
                let (envelope, _) =
                    pilotage_protocol::decode_envelope_length_delimited(bytes).ok()?;
                match envelope.payload {
                    Some(wire::envelope::Payload::ProfileActivation(activation)) => {
                        Some(activation)
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .expect("the first grant announces the control profile");
    assert_eq!(activation.session.map(|s| s.value), Some(7));
    assert_eq!(activation.activation_revision, 1);
    assert_eq!(activation.digest.len(), 32);

    // Every later frame binds to the announced activation, or the host
    // rejects the press with "activation revision does not match".
    let actions = engine.control_frame(ControlCommand::Legacy(wire::ControlPayload::default()), 10);
    let ClientAction::SendDatagram(bytes) = &actions[0] else {
        panic!("a held lease produces a datagram");
    };
    let envelope = wire::Envelope::decode(bytes.as_slice()).expect("frame decodes");
    let Some(wire::envelope::Payload::ControlFrame(frame)) = envelope.payload else {
        panic!("the datagram is a control frame");
    };
    assert_eq!(frame.activation_revision, 1);
}

#[test]
fn a_video_stream_yields_frame_bodies_across_split_reads() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.handle(TransportEvent::UniStreamOpened(StreamId(3)), 0);

    let body = vec![0xAB_u8; 10];
    let mut wire_bytes = vec![0x04];
    wire_bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    wire_bytes.extend_from_slice(&body);

    let first = engine.handle(
        TransportEvent::UniStreamReceived(StreamId(3), wire_bytes[..6].to_vec()),
        0,
    );
    assert!(first.is_empty(), "a partial record emits nothing");
    let rest = engine.handle(
        TransportEvent::UniStreamReceived(StreamId(3), wire_bytes[6..].to_vec()),
        0,
    );
    let ClientAction::Emit(ModuleEvent::VideoFrame(received)) = &rest[0] else {
        panic!("a completed record emits one video frame body");
    };
    assert_eq!(received, &body);
}
