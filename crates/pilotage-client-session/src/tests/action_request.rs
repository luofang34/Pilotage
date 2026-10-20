use pilotage_protocol::wire;

use super::{admit, engine, grant, grant_scope};
use crate::{
    ActionRequestIdentity, ClientAction, ClientEngine, ModuleEvent, PreparedControlAction,
    StreamId, TransportEvent,
};

const MOTION: &str = "vehicle.motion";

fn takeoff(action_id: u32) -> wire::ControlActionRequest {
    wire::ControlActionRequest {
        action: wire::ControlAction::Takeoff as i32,
        action_id,
        ..Default::default()
    }
}

fn command(actions: &[ClientAction]) -> wire::ControlActionCommand {
    actions
        .iter()
        .find_map(|action| {
            let ClientAction::SendBootstrap(bytes) = action else {
                return None;
            };
            let (envelope, _) = pilotage_protocol::decode_envelope_length_delimited(bytes)
                .expect("prepared reliable command decodes");
            match envelope.payload {
                Some(wire::envelope::Payload::ControlActionCommand(command)) => Some(command),
                _ => None,
            }
        })
        .expect("one reliable action was prepared")
}

fn assert_identity(prepared: &PreparedControlAction, sampled_at_nanos: u64) {
    let sent = command(&prepared.actions);
    let identity = ActionRequestIdentity {
        session_id: sent.session.expect("session is encoded").value,
        vehicle_id: sent.vehicle.expect("vehicle is encoded").value,
        scope: sent.scope.expect("scope is encoded").value,
        generation: sent.generation.expect("generation is encoded").value,
        sequence: 0,
        action_id: sent.request.expect("request is encoded").action_id,
    };
    assert_eq!(prepared.identity, identity);
    assert_ne!(identity.action_id, 0);
    assert_eq!(
        sent.sampled_at.expect("input time is encoded").nanos,
        sampled_at_nanos
    );
}

fn revoke(engine: &mut ClientEngine, generation: u64) {
    let envelope = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::AuthorityEvent(
            wire::AuthorityEvent {
                event: Some(wire::authority_event::Event::ScopeLeaseRevoked(
                    wire::ScopeLeaseRevoked {
                        vehicle: Some(wire::VehicleId { value: 1 }),
                        scope: Some(wire::ScopeId {
                            value: MOTION.into(),
                        }),
                        generation: Some(wire::Generation { value: generation }),
                        ..Default::default()
                    },
                )),
            },
        )),
    };
    let mut bytes = vec![0x01];
    bytes.extend(pilotage_protocol::encode_envelope_length_delimited(
        &envelope,
    ));
    let stream = StreamId(generation);
    engine.handle(TransportEvent::UniStreamOpened(stream), 0);
    engine.handle(TransportEvent::UniStreamReceived(stream, bytes), 0);
}

#[test]
fn prepared_identity_uses_the_granted_lane_before_the_authority_event_arrives() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    assert!(
        engine
            .prepare_control_action_at(1, MOTION, takeoff(0), 10)
            .is_none()
    );
    engine.request_lease(1, MOTION);
    assert!(
        engine
            .prepare_control_action_at(1, MOTION, takeoff(0), 11)
            .is_none(),
        "a pending request does not open a lane"
    );
    grant(&mut engine, 4);
    assert!(engine.authority().holder(1, MOTION).is_none());

    let generated = engine
        .prepare_control_action_at(1, MOTION, takeoff(0), 12)
        .expect("the lease reply opened the lane");
    assert_identity(&generated, 12);
    assert_eq!(generated.identity.session_id, 7);
    assert_eq!(generated.identity.generation, 4);
    assert_eq!(generated.identity.action_id, 1);

    let explicit = engine
        .prepare_control_action_at(1, MOTION, takeoff(91), 13)
        .expect("the held lane prepares another request");
    assert_identity(&explicit, 13);
    assert_eq!(explicit.identity.action_id, 91);
}

#[test]
fn prepared_actions_share_ids_across_independent_lanes() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, MOTION);
    grant(&mut engine, 4);
    engine.request_lease_quiet(1, "vehicle.gimbal");
    grant_scope(&mut engine, "vehicle.gimbal", 9);
    let motion = engine
        .prepare_control_action_at(1, MOTION, takeoff(0), 20)
        .expect("motion lane held");
    let gimbal = engine
        .prepare_control_action_at(
            1,
            "vehicle.gimbal",
            wire::ControlActionRequest {
                action: wire::ControlAction::GimbalRecenter as i32,
                ..Default::default()
            },
            21,
        )
        .expect("gimbal lane held");
    assert_identity(&motion, 20);
    assert_identity(&gimbal, 21);
    assert_eq!(gimbal.identity.scope, "vehicle.gimbal");
    assert_eq!(gimbal.identity.generation, 9);
    assert_ne!(motion.identity.action_id, gimbal.identity.action_id);
    assert!(
        engine
            .prepare_control_action_at(2, MOTION, takeoff(0), 22)
            .is_none(),
        "a sibling vehicle cannot borrow the lane"
    );
}

#[test]
fn a_replacement_lease_prepares_its_own_identity_and_revocation_closes_it() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, MOTION);
    grant(&mut engine, 4);
    let first = engine
        .prepare_control_action_at(1, MOTION, takeoff(0), 30)
        .expect("first lease held");

    engine.release_lease(1, MOTION);
    assert!(
        engine
            .prepare_control_action_at(1, MOTION, takeoff(0), 31)
            .is_none()
    );
    engine.request_lease(1, MOTION);
    grant(&mut engine, 6);
    revoke(&mut engine, 5);
    let second = engine
        .prepare_control_action_at(1, MOTION, takeoff(0), 32)
        .expect("an older revocation does not close the new lane");
    assert_identity(&first, 30);
    assert_identity(&second, 32);
    assert_eq!(second.identity.generation, 6);
    assert_ne!(first.identity.action_id, second.identity.action_id);

    revoke(&mut engine, 7);
    assert!(
        engine
            .prepare_control_action_at(1, MOTION, takeoff(0), 33)
            .is_none()
    );
    assert!(
        engine
            .control_action_at(1, MOTION, takeoff(0), 33)
            .is_empty()
    );
}

#[test]
fn transport_failure_removes_action_authority_and_reconnect_does_not_replay() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, MOTION);
    grant(&mut engine, 4);
    let prepared = engine
        .prepare_control_action_at(1, MOTION, takeoff(0), 40)
        .expect("lease held before transport failure");
    assert_identity(&prepared, 40);

    let lost = engine.handle(
        TransportEvent::TransportLost {
            detail: "bootstrap write failed".into(),
        },
        41,
    );
    assert!(lost.iter().any(|action| matches!(
        action,
        ClientAction::Emit(ModuleEvent::ConnectionDown { .. })
    )));
    assert!(
        engine
            .prepare_control_action_at(1, MOTION, takeoff(0), 42)
            .is_none()
    );
    let recovered = admit(&mut engine, 8, 42);
    assert!(
        !recovered
            .iter()
            .any(|action| matches!(action, ClientAction::SendBootstrap(_))),
        "admission does not replay a prepared command"
    );
    assert!(
        engine
            .prepare_control_action_at(1, MOTION, takeoff(0), 43)
            .is_none(),
        "reconnect restores observation without a control lease"
    );
    engine.request_lease(1, MOTION);
    grant(&mut engine, 10);
    let fresh = engine
        .prepare_control_action_at(1, MOTION, takeoff(0), 44)
        .expect("operator requested a new lease");
    assert_identity(&fresh, 44);
    assert_eq!(fresh.identity.session_id, 8);
    assert_eq!(fresh.identity.generation, 10);
}

#[test]
fn the_compatible_entry_point_uses_the_same_prepared_command() {
    let mut engine = engine();
    let mut compatible = super::engine();
    for client in [&mut engine, &mut compatible] {
        admit(client, 7, 42);
        client.request_lease(1, MOTION);
        grant(client, 4);
    }
    let prepared = engine
        .prepare_control_action_at(1, MOTION, takeoff(0), 50)
        .expect("held lane");
    let actions = compatible.control_action_at(1, MOTION, takeoff(0), 50);
    assert_eq!(prepared.actions, actions);
}
