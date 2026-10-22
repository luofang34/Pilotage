//! Authority groups on the client: the host holds both flight scopes as
//! one authority and names the group (`vehicle.motion`) in its events, so
//! a group revocation, override, or handover ends the direct lane too.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;

use super::{engine, grant_scope};
use crate::{ClientAction, ClientEngine, ModuleEvent, StreamId, TransportEvent};

const MOTION: &str = "vehicle.motion";
const DIRECT: &str = "vehicle.motion.direct";

/// Admits a multicopter whose two flight scopes share the motion group.
fn admit_grouped(engine: &mut ClientEngine) {
    let scope = |name: &str| wire::ScopeDescriptor {
        payload: None,
        scope: Some(wire::ScopeId { value: name.into() }),
        authority_group: MOTION.into(),
        ..Default::default()
    };
    let welcome = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::ServerWelcome(
            wire::ServerWelcome {
                session: Some(wire::SessionId { value: 7 }),
                principal: Some(wire::PrincipalId { value: 42 }),
                host_capabilities: Some(wire::HostCapabilities {
                    control_window_ms: 0,
                    host_version: "test-host".into(),
                    vehicles: vec![wire::VehicleDescriptor {
                        vehicle: Some(wire::VehicleId { value: 1 }),
                        display_name: "quad".into(),
                        scopes: vec![
                            scope(MOTION),
                            scope(DIRECT),
                            wire::ScopeDescriptor {
                                payload: None,
                                scope: Some(wire::ScopeId {
                                    value: "vehicle.gimbal".into(),
                                }),
                                ..Default::default()
                            },
                        ],
                        supported_modes: Vec::new(),
                        vehicle_class: wire::VehicleClass::Multicopter as i32,
                        heading_reference: 0,
                    }],
                    supported_modes: Vec::new(),
                    control_feel: None,
                    physically_embodied: false,
                }),
                scope_holders: Vec::new(),
            },
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&welcome);
    engine.handle(TransportEvent::Connected, 0);
    engine.handle(TransportEvent::BootstrapReceived(bytes), 0);
}

/// An engine holding the direct scope at generation 4.
fn holding_direct() -> ClientEngine {
    let mut engine = engine();
    admit_grouped(&mut engine);
    engine.request_lease(1, DIRECT);
    grant_scope(&mut engine, DIRECT, 4);
    assert!(engine.holds(1, DIRECT));
    engine
}

fn deliver(
    engine: &mut ClientEngine,
    stream: u64,
    event: wire::authority_event::Event,
) -> Vec<ClientAction> {
    let envelope = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::AuthorityEvent(
            wire::AuthorityEvent { event: Some(event) },
        )),
    };
    let mut tagged = vec![0x01];
    tagged.extend_from_slice(&pilotage_protocol::encode_envelope_length_delimited(
        &envelope,
    ));
    engine.handle(TransportEvent::UniStreamOpened(StreamId(stream)), 0);
    engine.handle(
        TransportEvent::UniStreamReceived(StreamId(stream), tagged),
        0,
    )
}

fn group_revoked(generation: u64) -> wire::authority_event::Event {
    wire::authority_event::Event::ScopeLeaseRevoked(wire::ScopeLeaseRevoked {
        vehicle: Some(wire::VehicleId { value: 1 }),
        scope: Some(wire::ScopeId {
            value: MOTION.into(),
        }),
        generation: Some(wire::Generation { value: generation }),
        ..Default::default()
    })
}

fn reports_direct_lost(actions: &[ClientAction]) -> bool {
    actions.iter().any(|action| {
        matches!(action, ClientAction::Emit(ModuleEvent::ControlLaneClosed { scope, .. })
            if scope == DIRECT)
    })
}

#[test]
fn the_catalog_carries_each_scopes_authority_group() {
    let mut engine = engine();
    admit_grouped(&mut engine);
    let admission = engine.admission().expect("admitted");
    assert_eq!(admission.authority_group_of(1, DIRECT), MOTION);
    assert_eq!(admission.authority_group_of(1, MOTION), MOTION);
    assert_eq!(
        admission.authority_group_of(1, "vehicle.gimbal"),
        "vehicle.gimbal",
        "an ungrouped scope is its own group"
    );
}

#[test]
fn a_group_revocation_ends_the_direct_lane() {
    let mut engine = holding_direct();
    let actions = deliver(&mut engine, 9, group_revoked(5));
    assert!(
        !engine.holds(1, DIRECT),
        "the motion group's revocation ends direct"
    );
    assert!(reports_direct_lost(&actions), "and says so: {actions:?}");
}

#[test]
fn an_older_group_revocation_leaves_the_direct_lane_open() {
    let mut engine = holding_direct();
    let actions = deliver(&mut engine, 9, group_revoked(3));
    assert!(engine.holds(1, DIRECT));
    assert!(!reports_direct_lost(&actions));
}

#[test]
fn an_override_by_another_operator_ends_the_direct_lane() {
    let mut engine = holding_direct();
    let override_applied =
        wire::authority_event::Event::EmergencyOverrideApplied(wire::EmergencyOverrideApplied {
            principal: Some(wire::PrincipalId { value: 99 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: MOTION.into(),
            }),
            generation: Some(wire::Generation { value: 5 }),
            previous_holder: Some(wire::PrincipalId { value: 42 }),
            ..Default::default()
        });
    let actions = deliver(&mut engine, 9, override_applied);
    assert!(!engine.holds(1, DIRECT));
    assert!(reports_direct_lost(&actions), "{actions:?}");
}

#[test]
fn a_handover_of_the_group_to_another_operator_ends_the_direct_lane() {
    let mut engine = holding_direct();
    let committed =
        wire::authority_event::Event::ScopeTransferCommitted(wire::ScopeTransferCommitted {
            from_principal: Some(wire::PrincipalId { value: 42 }),
            to_principal: Some(wire::PrincipalId { value: 99 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: MOTION.into(),
            }),
            generation: Some(wire::Generation { value: 5 }),
            ..Default::default()
        });
    let actions = deliver(&mut engine, 9, committed);
    assert!(!engine.holds(1, DIRECT));
    assert!(reports_direct_lost(&actions), "{actions:?}");
}

#[test]
fn a_revocation_of_another_group_leaves_the_flight_lanes_alone() {
    let mut engine = holding_direct();
    let gimbal = wire::authority_event::Event::ScopeLeaseRevoked(wire::ScopeLeaseRevoked {
        vehicle: Some(wire::VehicleId { value: 1 }),
        scope: Some(wire::ScopeId {
            value: "vehicle.gimbal".into(),
        }),
        generation: Some(wire::Generation { value: 9 }),
        ..Default::default()
    });
    deliver(&mut engine, 9, gimbal);
    assert!(engine.holds(1, DIRECT));
}
