//! The `sim.lifecycle` scope is COMMAND-ONLY (SIM-01): it carries no
//! continuous setpoints, so it must not arm the frame-silence watchdog, and
//! releasing it must not engage link loss — the scope has no neutral intent
//! capable of clearing a latch, so an engaged brake would reject every
//! subsequent reset forever. The reset -> release -> re-lease cycle must
//! stay repeatable.

use pilotage_adapter_api::{SIM_LIFECYCLE_SCOPE, sim_lifecycle_descriptor};
use pilotage_protocol::{
    ControlAction, ControlActionCommand, Generation, LeaseRelease, LeaseRequest, ScopeId, SessionId,
};
use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engaged_neutralize, link_lost, staleness, welcome};
use crate::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
};

/// The default (typed-only, production) config over capabilities that also
/// advertise the lifecycle scope — the simulation-adapter shape.
fn lifecycle_engine() -> SessionEngine {
    let mut capabilities = super::capabilities();
    capabilities.vehicles[0]
        .scopes
        .push(sim_lifecycle_descriptor());
    SessionEngine::new(
        capabilities,
        staleness(),
        SessionConfig::new(1, "host-test"),
    )
}

fn lifecycle_scope() -> ScopeId {
    ScopeId::new(SIM_LIFECYCLE_SCOPE)
}

fn announce(engine: &mut SessionEngine, client: ClientKey, session: SessionId) {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::ProfileActivation(pilotage_protocol::ProfileActivation {
            session,
            profile_id: "builtin.flight.default".to_owned(),
            profile_revision: 1,
            activation_revision: 1,
            digest: [0x11; 32],
            device_profile_id: String::new(),
            device_profile_revision: 0,
            device_digest: [0; 32],
        }),
        MonoTimestamp::from_nanos(1),
    );
    assert!(matches!(
        outcome.actions.as_slice(),
        [SessionAction::ActivationAccepted { .. }]
    ));
}

fn lease_lifecycle(engine: &mut SessionEngine, client: ClientKey, now: u64) -> Generation {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: lifecycle_scope(),
        }),
        MonoTimestamp::from_nanos(now),
    );
    outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            } if response.granted => Some(response.generation),
            _ => None,
        })
        .expect("lifecycle lease granted")
}

fn reset_command(session: SessionId, generation: Generation, action_id: u32) -> DomainEnvelope {
    DomainEnvelope::ActionCommand(ControlActionCommand {
        session,
        vehicle: VEHICLE,
        scope: lifecycle_scope(),
        generation,
        activation_revision: 1,
        action: ControlAction::SimReset,
        action_id,
    })
}

/// Two COMPLETE reset -> release -> re-lease cycles: each cycle's reset is
/// delivered, no cycle arms the watchdog, and no release engages link loss.
#[test]
fn the_reset_release_re_lease_cycle_stays_repeatable() {
    let mut engine = lifecycle_engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session);

    let mut previous = Generation::new(0);
    for cycle in 0u64..2 {
        let base = 100 * cycle + 10;
        let generation = lease_lifecycle(&mut engine, client, base);
        assert!(
            generation.as_u64() > previous.as_u64(),
            "cycle {cycle}: each lease lands strictly newer"
        );
        assert_eq!(
            engine.next_deadline(),
            None,
            "cycle {cycle}: a command-only scope must not arm the frame-silence watchdog"
        );

        let action_id = u32::try_from(cycle).expect("small cycle index") + 1;
        let delivered = engine.handle_client_message(
            client,
            reset_command(session, generation, action_id),
            MonoTimestamp::from_nanos(base + 1),
        );
        assert!(
            delivered
                .actions
                .iter()
                .any(|action| matches!(action, SessionAction::ApplyToAdapter { .. })),
            "cycle {cycle}: the reset must reach the adapter — a latched brake would reject it: {:?}",
            delivered.actions
        );

        let released = engine.handle_client_message(
            client,
            DomainEnvelope::Release(LeaseRelease {
                vehicle: VEHICLE,
                scope: lifecycle_scope(),
            }),
            MonoTimestamp::from_nanos(base + 2),
        );
        assert_eq!(
            engaged_neutralize(&released),
            0,
            "cycle {cycle}: releasing a command-only scope must not engage link loss: {:?}",
            released.actions
        );
        assert!(
            !link_lost(&released),
            "cycle {cycle}: no HolderLinkLost broadcast on a command-only release: {:?}",
            released.actions
        );
        previous = generation;
    }
}
