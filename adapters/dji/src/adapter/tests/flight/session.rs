use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_protocol::{
    ClientHello, CommandTiming, ControlAction, ControlActionCommand, Generation, LeaseRequest,
    Ping, ProfileActivation, ScopeId, SessionId, VehicleId,
};
use pilotage_session::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
    SessionOutcome,
};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

use super::*;

struct Session {
    io: Harness,
    engine: SessionEngine,
    id: SessionId,
    generation: Generation,
    window: u64,
}

impl Session {
    fn new() -> Self {
        let io = ready();
        let engine = SessionEngine::new(
            io.adapter.capabilities(),
            StalenessPolicy::new(Duration::from_millis(250)),
            SessionConfig::new(1, "dji-flight-test"),
        );
        let mut test = Self {
            io,
            engine,
            id: SessionId::new(0),
            generation: Generation::new(0),
            window: 0,
        };
        test.establish();
        test
    }

    fn establish(&mut self) {
        let hello = self.message(
            DomainEnvelope::Hello(ClientHello {
                protocol_version: 1,
                client_name: "discrete-test".to_owned(),
                join_token: Vec::new(),
            }),
            11,
        );
        self.id = hello
            .actions
            .iter()
            .find_map(|action| match action {
                SessionAction::SendToClient {
                    envelope: OutboundMessage::Welcome(welcome),
                    ..
                } => Some(welcome.session),
                _ => None,
            })
            .unwrap();
        let activated = self.message(
            DomainEnvelope::ProfileActivation(ProfileActivation {
                session: self.id,
                profile_id: "builtin.gimbal".to_owned(),
                profile_revision: 1,
                activation_revision: 1,
                digest: [1; 32],
                device_profile_id: String::new(),
                device_profile_revision: 0,
                device_digest: [0; 32],
            }),
            12,
        );
        assert!(matches!(
            activated.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ));
        let lease = self.message(
            DomainEnvelope::Lease(LeaseRequest {
                vehicle: VehicleId::new(1),
                scope: ScopeId::new(FLIGHT_SCOPE),
            }),
            13,
        );
        self.generation = lease
            .actions
            .iter()
            .find_map(|action| match action {
                SessionAction::SendToClient {
                    envelope: OutboundMessage::LeaseResponse(lease),
                    ..
                } if lease.granted => Some(lease.generation),
                _ => None,
            })
            .unwrap();
        let ping = self.message(
            DomainEnvelope::Ping(Ping {
                nonce: 1,
                sender_sent_at: stamp(14),
            }),
            14,
        );
        self.window = ping
            .actions
            .iter()
            .find_map(|action| match action {
                SessionAction::SendToClient {
                    envelope: OutboundMessage::Pong(pong),
                    ..
                } => Some(pong.control_window_id),
                _ => None,
            })
            .unwrap();
    }

    fn message(&mut self, message: DomainEnvelope, ms: u64) -> SessionOutcome {
        self.engine
            .handle_client_message(ClientKey::new(1), message, stamp(ms))
    }

    fn action(&mut self, action: ControlAction, id: u32, ms: u64) -> SessionOutcome {
        self.message(
            DomainEnvelope::ActionCommand(ControlActionCommand {
                sampled_at: stamp(ms),
                timing: Some(CommandTiming {
                    control_window_id: self.window,
                    require_control_window: true,
                    ..CommandTiming::default()
                }),
                session: self.id,
                vehicle: VehicleId::new(1),
                scope: ScopeId::new(FLIGHT_SCOPE),
                generation: self.generation,
                activation_revision: 1,
                action,
                action_id: id,
            }),
            ms,
        )
    }

    fn enact(&mut self, outcome: SessionOutcome, ms: u64) {
        for action in outcome.actions {
            match action {
                SessionAction::ApplyToAdapter { frame, .. } => {
                    assert_eq!(self.io.apply(frame, ms).disposition, Disposition::Accepted);
                }
                SessionAction::InvalidatePendingCommands { vehicle, scope } => {
                    self.io
                        .adapter
                        .invalidate_scope(vehicle, &scope, Duration::from_millis(ms));
                }
                SessionAction::EngageLinkLoss { .. } | SessionAction::ClearLinkLoss { .. } => {
                    panic!("command-only flight must not synthesize link-loss actuation");
                }
                _ => {}
            }
        }
    }
}

fn stamp(ms: u64) -> MonoTimestamp {
    MonoTimestamp::from_nanos(ms * 1_000_000)
}

#[test]
fn real_command_only_lease_disconnect_revokes_queued_takeoff_without_motor_write() {
    let mut test = Session::new();
    let admitted = test.action(ControlAction::Takeoff, 41, 20);
    assert!(
        admitted
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::ApplyToAdapter { .. }))
    );
    test.enact(admitted, 20);
    let lost = test.message(DomainEnvelope::Disconnect, 21);
    assert!(
        lost.actions
            .iter()
            .any(|action| matches!(action, SessionAction::InvalidatePendingCommands { .. }))
    );
    test.enact(lost, 21);
    test.io.step(30);
    assert!(commands(&test.io).is_empty());
    assert_eq!(
        progress(&mut test.io).phase,
        pilotage_adapter_api::FlightProcedurePhase::Cancelled
    );
}

#[test]
fn real_command_only_lease_is_not_silence_neutralized_after_written_action() {
    let mut test = Session::new();
    let admitted = test.action(ControlAction::Takeoff, 41, 20);
    test.enact(admitted, 20);
    test.io.step(30);
    let tick = test.engine.handle_tick(stamp(1_100));
    assert!(!tick.actions.iter().any(|action| matches!(
        action,
        SessionAction::EngageLinkLoss { .. } | SessionAction::InvalidatePendingCommands { .. }
    )));
    test.enact(tick, 1_100);
    assert_eq!(commands(&test.io).len(), 1);
}
