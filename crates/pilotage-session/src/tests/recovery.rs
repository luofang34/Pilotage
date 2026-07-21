//! Recovery activation gate (ADR-0008): after a link-loss policy engages, a
//! fresh fenced generation alone must not clear it — the new holder must also
//! land an accepted neutral frame, so deflected sticks on reconnect cannot revive motion.

use core::time::Duration;

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_authority::{AuthorityClass, AuthorityEffect, OverrideReason};
use pilotage_protocol::{
    ControlPayload, Generation, LeaseRequest, LogicalAxisId, PrincipalId, ScopeId,
    ScopedControlFrame, SequenceNum, SessionId,
};
use pilotage_timing::MonoTimestamp;

use super::{
    VEHICLE, cleared, edge_only_frame, engaged_neutralize, engine_with_silence, frame, grant,
    motion, neutral_frame, staleness, welcome,
};
use crate::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
};

/// An engine whose holder went silent (policy engaged), then re-granted the
/// scope to the same client at a fresh generation.
fn engaged_then_regranted() -> (SessionEngine, ClientKey, SessionId, Generation) {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    grant(&mut engine, client, 1); // deadline 101
    let lost = engine.handle_tick(MonoTimestamp::from_nanos(101));
    assert_eq!(engaged_neutralize(&lost), 1);
    let generation = grant(&mut engine, client, 200);
    (engine, client, session, generation)
}

#[test]
fn a_grant_alone_does_not_clear_neutralization() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    grant(&mut engine, client, 1); // deadline 101
    let lost = engine.handle_tick(MonoTimestamp::from_nanos(101));
    assert_eq!(engaged_neutralize(&lost), 1);

    let regrant = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(200),
    );
    assert_eq!(
        cleared(&regrant),
        0,
        "a grant alone must not clear link-loss: {:?}",
        regrant.actions
    );
}

#[test]
fn deflected_or_edge_frames_do_not_activate_recovery() {
    let (mut engine, client, session, generation) = engaged_then_regranted();

    // Deflected sticks (the shared fixture's 0.25 axis) do not activate.
    let deflected = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(frame(
            session,
            generation,
            SequenceNum::new(1),
            MonoTimestamp::from_nanos(210),
        )),
        MonoTimestamp::from_nanos(210),
    );
    assert_eq!(
        cleared(&deflected),
        0,
        "deflected sticks must not clear link-loss: {:?}",
        deflected.actions
    );

    // An edge-only frame is not a neutral demonstration either.
    let edged = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(edge_only_frame(
            session,
            generation,
            SequenceNum::new(2),
            MonoTimestamp::from_nanos(220),
        )),
        MonoTimestamp::from_nanos(220),
    );
    assert_eq!(cleared(&edged), 0, "edges cannot activate recovery");
}

#[test]
fn a_neutral_frame_clears_once_and_orders_before_the_apply() {
    let (mut engine, client, session, generation) = engaged_then_regranted();

    // Demonstrated-neutral sticks clear it, exactly once, with the clear
    // ordered before the frame's apply so the adapter un-latches first.
    let neutral = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(neutral_frame(
            session,
            generation,
            SequenceNum::new(3),
            MonoTimestamp::from_nanos(230),
        )),
        MonoTimestamp::from_nanos(230),
    );
    assert_eq!(
        cleared(&neutral),
        1,
        "neutral frame clears: {:?}",
        neutral.actions
    );
    let clear_index = neutral
        .actions
        .iter()
        .position(|a| matches!(a, SessionAction::ClearLinkLoss { .. }))
        .expect("clear present");
    let apply_index = neutral
        .actions
        .iter()
        .position(|a| matches!(a, SessionAction::ApplyToAdapter { .. }))
        .expect("apply present");
    assert!(clear_index < apply_index, "clear must precede the apply");

    // The engine defers the client-facing ack to the driver: it emits a
    // ClearLinkLoss carrying the recovered generation, and the driver
    // broadcasts LinkLossCleared only AFTER the adapter confirms the clear
    // (a failed clear must NOT ack — see the engine-actor tests). Here we
    // assert the engine hands the driver the exact generation to echo.
    let carries_generation = neutral.actions.iter().any(|a| {
        matches!(
            a,
            SessionAction::ClearLinkLoss { generation: g, .. } if *g == generation
        )
    });
    assert!(
        carries_generation,
        "ClearLinkLoss must carry the recovered generation for the ack: {:?}",
        neutral.actions
    );

    // Recovery is complete; further neutral frames do not re-clear.
    let again = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(neutral_frame(
            session,
            generation,
            SequenceNum::new(4),
            MonoTimestamp::from_nanos(240),
        )),
        MonoTimestamp::from_nanos(240),
    );
    assert_eq!(cleared(&again), 0, "recovery already complete");
}

/// A capability report for one vehicle with the given scopes and link-loss menu.
fn profile(
    scopes: Vec<(&'static str, Vec<u16>)>,
    actions: Vec<LinkLossPolicy>,
) -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            deterministic: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VEHICLE,
            scopes: scopes
                .into_iter()
                .map(|(scope, axes)| {
                    // Route the declared axes onto velocity components in
                    // order, at unit limits, so legacy payload fixtures
                    // exercise the command gate's translation boundary.
                    let route = |index: usize| {
                        axes.get(index)
                            .map(|axis| pilotage_adapter_api::LegacyAxisRoute {
                                axis: *axis,
                                sign: 1.0,
                            })
                    };
                    ScopeDescriptor {
                        scope: ScopeId::new(scope),
                        axes: axes.iter().copied().map(LogicalAxisId::new).collect(),
                        intents: vec![pilotage_adapter_api::IntentCapability {
                            family: pilotage_protocol::IntentFamily::Velocity,
                            frames: vec![pilotage_protocol::ReferenceFrame::BodyFrd],
                            max_linear: 1.0,
                            max_vertical: 0.0,
                            max_angular: 1.0,
                        }],
                        actions: vec![],
                        legacy: Some(pilotage_adapter_api::LegacyCommandMap::Velocity {
                            vx: route(0),
                            vy: route(1),
                            vz: route(2),
                            yaw_rate: route(3),
                            arm_button: None,
                            disarm_button: None,
                            reset_button: None,
                        }),
                    }
                })
                .collect(),
            link_loss_actions: actions,
        }],
        adapter_version: "test".to_owned(),
    }
}

/// A control frame for an arbitrary scope with explicit axis values.
fn scoped_frame(
    session: SessionId,
    scope: &str,
    generation: Generation,
    sequence: u32,
    at: u64,
    axes: Vec<(u16, f32)>,
) -> ScopedControlFrame {
    ScopedControlFrame {
        session,
        vehicle: VEHICLE,
        scope: ScopeId::new(scope),
        generation,
        sequence: SequenceNum::new(sequence),
        sampled_at: MonoTimestamp::from_nanos(at),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload {
            axes: axes
                .into_iter()
                .map(|(a, v)| (LogicalAxisId::new(a), v))
                .collect(),
            edges: Vec::new(),
        },
        intent: None,
        actions: vec![],
    }
}

fn grant_scope(engine: &mut SessionEngine, client: ClientKey, scope: &str, now: u64) -> Generation {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: ScopeId::new(scope),
        }),
        MonoTimestamp::from_nanos(now),
    );
    match outcome.actions.last() {
        Some(SessionAction::SendToClient {
            envelope: OutboundMessage::LeaseResponse(response),
            ..
        }) if response.granted => response.generation,
        other => panic!("expected a granted lease for {scope}, got {other:?}"),
    }
}

#[test]
fn recovery_is_scope_specific_not_vehicle_wide() {
    // Two scopes on one vehicle, held by different clients. Losing scope
    // m1 engages the vehicle policy; a neutral frame on the UNRELATED
    // scope m2 proves nothing about m1 and must not clear it. Only a
    // fresh grant + neutral demonstration on m1 itself recovers.
    let mut engine = SessionEngine::new(
        profile(
            vec![("vehicle.m1", vec![0]), ("vehicle.m2", vec![0])],
            vec![LinkLossPolicy::Neutralize],
        ),
        staleness(),
        SessionConfig::new(1, "host-test").with_holder_silence(Duration::from_nanos(100)),
    );
    let (c1, c2) = (ClientKey::new(1), ClientKey::new(2));
    let s1 = welcome(&mut engine, c1);
    let s2 = welcome(&mut engine, c2);
    grant_scope(&mut engine, c1, "vehicle.m1", 1); // deadline 101
    let g2 = grant_scope(&mut engine, c2, "vehicle.m2", 2);

    // m2's holder keeps driving; only m1 goes silent and is lost.
    let refresh = engine.handle_client_message(
        c2,
        DomainEnvelope::Frame(scoped_frame(s2, "vehicle.m2", g2, 1, 95, vec![(0, 0.4)])),
        MonoTimestamp::from_nanos(95),
    );
    assert!(
        refresh
            .actions
            .iter()
            .any(|a| matches!(a, SessionAction::ApplyToAdapter { .. })),
        "m2 frame accepted: {:?}",
        refresh.actions
    );
    let lost = engine.handle_tick(MonoTimestamp::from_nanos(101));
    assert_eq!(engaged_neutralize(&lost), 1, "m1 loss engages the vehicle");

    // A neutral frame on m2 must NOT clear the m1 engagement.
    let unrelated = engine.handle_client_message(
        c2,
        DomainEnvelope::Frame(scoped_frame(s2, "vehicle.m2", g2, 2, 120, vec![(0, 0.0)])),
        MonoTimestamp::from_nanos(120),
    );
    assert_eq!(
        cleared(&unrelated),
        0,
        "an unrelated scope cannot activate recovery: {:?}",
        unrelated.actions
    );

    // Recovery on m1 itself: fresh grant, then a neutral m1 frame.
    let g1 = grant_scope(&mut engine, c1, "vehicle.m1", 200);
    let neutral = engine.handle_client_message(
        c1,
        DomainEnvelope::Frame(scoped_frame(s1, "vehicle.m1", g1, 3, 210, vec![(0, 0.0)])),
        MonoTimestamp::from_nanos(210),
    );
    assert_eq!(
        cleared(&neutral),
        1,
        "m1 recovery clears: {:?}",
        neutral.actions
    );
}

#[test]
fn partial_axis_coverage_cannot_activate_recovery() {
    // The scope declares two axes; adapters retain latest-valid values per
    // axis, so a frame omitting one axis could un-latch straight into a
    // stale deflection. Activation requires EVERY declared axis reported
    // neutral.
    let mut engine = SessionEngine::new(
        profile(
            vec![("vehicle.m1", vec![0, 1])],
            vec![LinkLossPolicy::Neutralize],
        ),
        staleness(),
        SessionConfig::new(1, "host-test").with_holder_silence(Duration::from_nanos(100)),
    );
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    grant_scope(&mut engine, client, "vehicle.m1", 1);
    let lost = engine.handle_tick(MonoTimestamp::from_nanos(101));
    assert_eq!(engaged_neutralize(&lost), 1);
    let generation = grant_scope(&mut engine, client, "vehicle.m1", 200);

    // Only axis 0 reported neutral: axis 1's stale value could revive.
    let partial = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(scoped_frame(
            session,
            "vehicle.m1",
            generation,
            1,
            210,
            vec![(0, 0.0)],
        )),
        MonoTimestamp::from_nanos(210),
    );
    assert_eq!(
        cleared(&partial),
        0,
        "partial axis coverage must not clear: {:?}",
        partial.actions
    );

    // Full declared coverage, all neutral: recovery completes.
    let full = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(scoped_frame(
            session,
            "vehicle.m1",
            generation,
            2,
            220,
            vec![(0, 0.0), (1, 0.0)],
        )),
        MonoTimestamp::from_nanos(220),
    );
    assert_eq!(
        cleared(&full),
        1,
        "full coverage clears: {:?}",
        full.actions
    );
}

#[test]
fn the_enacted_policy_is_configured_and_validated_never_order_based() {
    let cases: [(Vec<LinkLossPolicy>, Option<LinkLossPolicy>, LinkLossPolicy); 3] = [
        // Unconfigured: the floor, even when the menu declares only
        // another action — declaration order is a menu, not a selection.
        (
            vec![LinkLossPolicy::HoldBrief { ticks: 7 }],
            None,
            LinkLossPolicy::Neutralize,
        ),
        // Configured and declared: the configured policy is enacted.
        (
            vec![
                LinkLossPolicy::Neutralize,
                LinkLossPolicy::HoldBrief { ticks: 7 },
            ],
            Some(LinkLossPolicy::HoldBrief { ticks: 7 }),
            LinkLossPolicy::HoldBrief { ticks: 7 },
        ),
        // Configured but NOT declared: falls closed to the floor.
        (
            vec![LinkLossPolicy::Neutralize],
            Some(LinkLossPolicy::Brake),
            LinkLossPolicy::Neutralize,
        ),
    ];
    for (declared, configured, expected) in cases {
        let mut config =
            SessionConfig::new(1, "host-test").with_holder_silence(Duration::from_nanos(100));
        if let Some(policy) = configured {
            config = config.with_link_loss_policy(VEHICLE, policy);
        }
        let mut engine = SessionEngine::new(
            profile(vec![("vehicle.motion", vec![0])], declared),
            staleness(),
            config,
        );
        let client = ClientKey::new(1);
        welcome(&mut engine, client);
        grant(&mut engine, client, 1);
        let fired = engine.handle_tick(MonoTimestamp::from_nanos(101));
        let policy = fired.actions.iter().find_map(|action| match action {
            SessionAction::EngageLinkLoss { policy, .. } => Some(*policy),
            _ => None,
        });
        assert_eq!(policy, Some(expected), "configured {configured:?}");
    }
}

/// Drives the shared fixture to a scope PENDING an adapter clear: engaged after
/// silence, re-granted, then a neutral activation moves it to `ClearPending`
/// (the driver has not yet confirmed the adapter took the clear). Returns the
/// engine and the pending generation; a tick now retries the clear.
// The generation-race invalidation tests share this module's helpers but
// would push it past the file-size gate, so they live in a sibling
// submodule.
mod pending_clear;
