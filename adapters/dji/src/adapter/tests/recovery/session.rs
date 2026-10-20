use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_protocol::{
    ClientHello, CommandTiming, Generation, LeaseRelease, LeaseRequest, Ping, ProfileActivation,
    ScopeId, ScopedControlFrame, SequenceNum, SessionId, VehicleId,
};
use pilotage_session::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
    SessionOutcome,
};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

use super::super::super::GIMBAL_SCOPE;
use super::super::harness::{Harness, frame, observed_poses};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) struct RecoverySession {
    pub(super) io: Harness,
    engine: SessionEngine,
    session: SessionId,
    window: u64,
}

impl RecoverySession {
    pub(super) fn ready() -> Result<Self> {
        let io = Harness::new();
        let engine = SessionEngine::new(
            io.adapter.capabilities(),
            StalenessPolicy::new(Duration::from_millis(250)),
            SessionConfig::new(1, "dji-recovery"),
        );
        let mut test = Self {
            io,
            engine,
            session: SessionId::new(0),
            window: 0,
        };
        for (index, pose) in observed_poses().into_iter().enumerate() {
            let at = index as u64 * 10;
            test.io.feed(&pose)?;
            test.io.step(at);
            let changed = test.refresh(at);
            assert!(
                !changed
                    .actions
                    .iter()
                    .any(|action| matches!(action, SessionAction::EngageLinkLoss { .. }))
            );
            test.enact(changed, at)?;
        }
        let welcome = test.message(
            DomainEnvelope::Hello(ClientHello {
                protocol_version: 1,
                client_name: "recovery".to_owned(),
                join_token: Vec::new(),
            }),
            21,
        );
        test.session = welcome
            .actions
            .iter()
            .find_map(|action| match action {
                SessionAction::SendToClient {
                    envelope: OutboundMessage::Welcome(welcome),
                    ..
                } => Some(welcome.session),
                _ => None,
            })
            .ok_or("missing session welcome")?;
        let activation = test.message(
            DomainEnvelope::ProfileActivation(ProfileActivation {
                session: test.session,
                profile_id: "builtin.gimbal".to_owned(),
                profile_revision: 1,
                activation_revision: 1,
                digest: [0x11; 32],
                device_profile_id: String::new(),
                device_profile_revision: 0,
                device_digest: [0; 32],
            }),
            21,
        );
        assert!(matches!(
            activation.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ));
        test.renew_window(21)?;
        Ok(test)
    }

    pub(super) fn lease(&mut self, at: u64) -> Result<Generation> {
        let outcome = self.message(
            DomainEnvelope::Lease(LeaseRequest {
                vehicle: VehicleId::new(1),
                scope: ScopeId::new(GIMBAL_SCOPE),
            }),
            at,
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
            .ok_or_else(|| "missing granted lease".into())
    }

    pub(super) fn release(&mut self, at: u64) -> SessionOutcome {
        self.message(
            DomainEnvelope::Release(LeaseRelease {
                vehicle: VehicleId::new(1),
                scope: ScopeId::new(GIMBAL_SCOPE),
            }),
            at,
        )
    }

    pub(super) fn renew_window(&mut self, at: u64) -> Result<()> {
        let outcome = self.message(
            DomainEnvelope::Ping(Ping {
                nonce: at,
                sender_sent_at: stamp(at),
            }),
            at,
        );
        self.window = outcome
            .actions
            .iter()
            .find_map(|action| match action {
                SessionAction::SendToClient {
                    envelope: OutboundMessage::Pong(pong),
                    ..
                } => Some(pong.control_window_id),
                _ => None,
            })
            .ok_or("missing execution window")?;
        Ok(())
    }

    pub(super) fn request(
        &self,
        generation: Generation,
        sequence: u32,
        pitch_rate: f32,
        at: u64,
    ) -> ScopedControlFrame {
        let mut request = frame(pitch_rate);
        request.session = self.session;
        request.generation = generation;
        request.sequence = SequenceNum::new(sequence);
        request.sampled_at = stamp(at);
        request.timing = Some(CommandTiming {
            control_window_id: self.window,
            require_control_window: true,
            ..CommandTiming::default()
        });
        request.admitted_timing = None;
        request.profile_revision = 1;
        request.activation_revision = 1;
        request
    }

    pub(super) fn frame(
        &mut self,
        generation: Generation,
        sequence: u32,
        pitch_rate: f32,
        at: u64,
    ) -> SessionOutcome {
        self.message(
            DomainEnvelope::Frame(self.request(generation, sequence, pitch_rate, at)),
            at,
        )
    }

    pub(super) fn pose(&mut self, offset: u16, at: u64) -> Result<()> {
        let mut pose = observed_poses().remove(2);
        let sequence = u16::from_le_bytes([pose[6], pose[7]]).wrapping_add(offset);
        pose[6..8].copy_from_slice(&sequence.to_le_bytes());
        let length = pose.len();
        let crc = turret_dji::frame_checksum(&pose[..length - 2]).to_le_bytes();
        pose[length - 2..].copy_from_slice(&crc);
        self.io.feed(&pose)?;
        self.io.step(at);
        Ok(())
    }

    pub(super) fn refresh(&mut self, at: u64) -> SessionOutcome {
        self.engine
            .refresh_capabilities(self.io.adapter.capabilities(), stamp(at))
    }

    pub(super) fn enact(&mut self, outcome: SessionOutcome, at: u64) -> Result<Vec<Disposition>> {
        assert_eq!(outcome.dropped, 0);
        let mut applied = Vec::new();
        for action in outcome.actions {
            match action {
                SessionAction::EngageLinkLoss { policy, .. } => {
                    self.io.link_loss(Some(policy), at)?;
                }
                SessionAction::ClearLinkLoss {
                    vehicle,
                    scope,
                    generation,
                    ..
                } => {
                    self.io.link_loss(None, at)?;
                    assert!(
                        self.engine
                            .confirm_link_loss_cleared(vehicle, &scope, generation)
                    );
                }
                SessionAction::ApplyToAdapter { frame, .. } => {
                    applied.push(self.io.apply(frame, at).disposition);
                }
                _ => {}
            }
        }
        Ok(applied)
    }

    fn message(&mut self, message: DomainEnvelope, at: u64) -> SessionOutcome {
        self.engine
            .handle_client_message(ClientKey::new(1), message, stamp(at))
    }
}

fn stamp(millis: u64) -> MonoTimestamp {
    MonoTimestamp::from_nanos(millis * 1_000_000)
}
