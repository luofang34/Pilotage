//! Shared fixtures for the engine-actor tests: the capability report, the
//! stateful recording adapter honoring the ADR-0008 latch postcondition, and
//! the handshake/grant drivers.

use super::*;

pub(super) fn capabilities() -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            deterministic: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VEHICLE,
            scopes: vec![ScopeDescriptor {
                scope: ScopeId::new("vehicle.motion"),
                axes: vec![LogicalAxisId::new(0)],
                intents: vec![pilotage_adapter_api::IntentCapability {
                    max_yaw_rate: 0.0,
                    family: pilotage_protocol::IntentFamily::Velocity,
                    frames: vec![pilotage_protocol::ReferenceFrame::BodyFrd],
                    max_linear: 1.0,
                    max_vertical: 0.0,
                    max_angular: 1.0,
                }],
                actions: vec![],
                legacy: Some(pilotage_adapter_api::LegacyCommandMap::Velocity {
                    vx: Some(pilotage_adapter_api::LegacyAxisRoute { axis: 0, sign: 1.0 }),
                    vy: None,
                    vz: None,
                    yaw_rate: None,
                    arm_button: None,
                    disarm_button: None,
                    reset_button: None,
                }),
            }],
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
        }],
        adapter_version: "test".to_owned(),
    }
}

/// A STATEFUL vehicle adapter honoring the ADR-0008 latch postcondition: an
/// engage records the latch even when the actuation is refused, a clear removes
/// it ONLY on success, and `apply_control` rejects a latched scope (so a test
/// observes suppression). `fail_enactment` refuses every policy change.
#[derive(Default)]
pub(super) struct RecordingAdapter {
    pub(super) link_loss_calls: Vec<(VehicleId, ScopeId, Option<LinkLossPolicy>)>,
    /// Scopes whose latch is currently engaged (suppressing control).
    pub(super) latched: Vec<ScopeId>,
    pub(super) fail_enactment: bool,
    /// The discrete actions of every `apply_control` call, in call order —
    /// the exactly-once observable for the dedup tests.
    pub(super) applied_actions: Vec<Vec<pilotage_protocol::ControlAction>>,
}

impl VehicleAdapter for RecordingAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        capabilities()
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        self.applied_actions.push(frame.actions.clone());
        let disposition = if self.latched.contains(&frame.scope) {
            Disposition::Rejected(RejectReason::LinkLossEngaged)
        } else {
            Disposition::Accepted
        };
        ApplyOutcome {
            tick: SimTick::new(0),
            disposition,
            action_results: frame
                .actions
                .iter()
                .map(|action| pilotage_adapter_api::ActionResult {
                    action: *action,
                    accepted: true,
                    detail: String::new(),
                })
                .collect(),
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        TelemetryBatch {
            samples: Vec::new(),
        }
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        Vec::new()
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        self.link_loss_calls.push((vehicle, scope.clone(), policy));
        match policy {
            // Engage records the latch REGARDLESS of the actuation result.
            Some(_) => {
                if !self.latched.contains(scope) {
                    self.latched.push(scope.clone());
                }
                if self.fail_enactment {
                    return Err(LinkLossEnactError::NoActuationChannel);
                }
            }
            // Clear drops the latch ONLY on success (ADR-0008).
            None => {
                if self.fail_enactment {
                    return Err(LinkLossEnactError::NoActuationChannel);
                }
                self.latched.retain(|latched| latched != scope);
            }
        }
        Ok(())
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        StepOutcome {
            advanced: 0,
            now: SimTick::new(0),
        }
    }
}

pub(super) fn actor() -> EngineActor<RecordingAdapter> {
    let engine = SessionEngine::new(
        capabilities(),
        StalenessPolicy::new(std::time::Duration::from_millis(250)),
        SessionConfig::new(1, "host-test"),
    );
    EngineActor::new(engine, RecordingAdapter::default(), Instant::now())
}

pub(super) fn engage_action() -> SessionAction {
    SessionAction::EngageLinkLoss {
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
        generation: Generation::new(2),
        trigger: LinkLossTrigger::HolderSilence,
        policy: LinkLossPolicy::Neutralize,
    }
}

pub(super) fn clear_action(scope: &str) -> SessionAction {
    SessionAction::ClearLinkLoss {
        vehicle: VEHICLE,
        scope: ScopeId::new(scope),
        generation: Generation::new(3),
        retry: false,
    }
}

/// A full-coverage neutral motion frame for the given fencing metadata; the
/// single builder both the raw-frame and recovery-activation helpers use.
pub(super) fn motion_frame(
    session: SessionId,
    generation: Generation,
    sequence: u32,
) -> ScopedControlFrame {
    ScopedControlFrame {
        action_ids: vec![],
        session,
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
        generation,
        sequence: SequenceNum::new(sequence),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload {
            axes: vec![(LogicalAxisId::new(0), 0.0)],
            edges: Vec::new(),
        },
        intent: None,
        actions: vec![],
    }
}

/// A raw motion control frame, for driving `apply_control` directly.
pub(super) fn motion_control_frame() -> ScopedControlFrame {
    motion_frame(SessionId::new(1), Generation::new(1), 1)
}

/// Registers one client and returns its outbound receiver, so a test can
/// observe what the actor broadcasts.
pub(super) fn register_client(
    actor: &mut EngineActor<RecordingAdapter>,
) -> mpsc::Receiver<ToConnection> {
    let (sender, receiver) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
    actor.clients.insert(ClientKey::new(1), sender);
    receiver
}

/// Counts the reliable authority-stream broadcasts (the ack rides one).
pub(super) fn authority_messages(receiver: &mut mpsc::Receiver<ToConnection>) -> usize {
    let mut count = 0;
    while let Ok(message) = receiver.try_recv() {
        if matches!(message, ToConnection::AuthorityMessage(_)) {
            count += 1;
        }
    }
    count
}

pub(super) fn hello() -> DomainEnvelope {
    DomainEnvelope::Hello(ClientHello {
        protocol_version: 1,
        client_name: "host-test".to_owned(),
        join_token: Vec::new(),
    })
}

pub(super) fn lease() -> DomainEnvelope {
    DomainEnvelope::Lease(LeaseRequest {
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
    })
}

pub(super) fn release() -> DomainEnvelope {
    DomainEnvelope::Release(LeaseRelease {
        vehicle: VEHICLE,
        scope: ScopeId::new(MOTION),
    })
}

/// A full-coverage neutral motion frame — the host's recovery activation.
pub(super) fn neutral_frame(
    session: SessionId,
    generation: Generation,
    sequence: u32,
) -> DomainEnvelope {
    DomainEnvelope::Frame(motion_frame(session, generation, sequence))
}

pub(super) fn welcome_session(outcome: &SessionOutcome) -> SessionId {
    outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::Welcome(welcome),
                ..
            } => Some(welcome.session),
            _ => None,
        })
        .expect("a welcome")
}

pub(super) fn grant_generation(outcome: &SessionOutcome) -> Generation {
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
        .expect("a granted lease")
}

/// Drives `actor`'s engine to a fresh holder pending recovery (welcome, grant,
/// release — which latches the adapter — re-grant), enacting every outcome. The
/// client is always `ClientKey::new(1)` (the `register_client` one) so a
/// broadcast ack reaches its receiver.
pub(super) fn drive_to_regranted(
    actor: &mut EngineActor<RecordingAdapter>,
) -> (SessionId, Generation) {
    let client = ClientKey::new(1);
    let now = MonoTimestamp::from_nanos(0);
    let welcome = actor.engine.handle_client_message(client, hello(), now);
    let session = welcome_session(&welcome);
    actor.enact(welcome);
    let granted = actor.engine.handle_client_message(client, lease(), now);
    actor.enact(granted);
    let released = actor.engine.handle_client_message(client, release(), now);
    actor.enact(released); // engages link-loss → the adapter's motion latch is set
    let regranted = actor.engine.handle_client_message(client, lease(), now);
    let generation = grant_generation(&regranted);
    actor.enact(regranted);
    (session, generation)
}
