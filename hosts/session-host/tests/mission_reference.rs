//! In-process mission-executor integration test: the reference engine
//! actor and the mission principal wired exactly as the runtime wires
//! them for the reference adapter — no transport endpoint, no network.
//!
//! The planar skiff cannot traverse geodetic waypoints meaningfully, so
//! the test scopes to the fenced in-process path itself: the principal
//! completes the handshake, announces its activation, leases the motion
//! scope, arms over the reliable action path, publishes stamped
//! navigation guidance onto the telemetry the actor broadcasts, then
//! streams typed intent frames for 200 ticks with ZERO `FrameRejected`
//! and the lease still held. Every wait is event-driven on the status
//! watch or the broadcast datagrams under a bounded timeout — never a
//! sleep-and-poll.

#![cfg(feature = "sim")]
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_mission::MissionState;
use pilotage_protocol::{
    ClientHello, SESSION_PROTOCOL_VERSION, ScopeId, ScopeTransferAccept, ScopeTransferOffer,
    ScopeTransferRequest, VehicleId, wire,
};
use pilotage_session::DomainEnvelope;
use pilotage_session_host::runtime::{self, AutomationStatus, Participant, TelemetryObserver};
use prost::Message;
use tokio::time::timeout;

/// Virtual-time bound (the test runs under a paused clock, so this never
/// costs wall time); a stalled rig fails fast at this deadline.
const TEST_TIMEOUT: Duration = Duration::from_secs(120);

/// The fixture route's fixes, in fly order: guidance names one of these
/// or it is not describing the mission being flown.
const DEMO_IDENTS: [&str; 3] = ["DEMOA", "DEMOB", "DEMOC"];

/// Awaits the first status satisfying `predicate`, event-driven on the
/// watch channel.
async fn await_status(
    status: &mut tokio::sync::watch::Receiver<AutomationStatus>,
    what: &str,
    predicate: impl Fn(&AutomationStatus) -> bool,
) -> AutomationStatus {
    await_status_within(status, TEST_TIMEOUT, what, predicate).await
}

/// [`await_status`] with its own virtual-time bound, for a flight that is
/// longer than the arming handshake.
async fn await_status_within(
    status: &mut tokio::sync::watch::Receiver<AutomationStatus>,
    bound: Duration,
    what: &str,
    predicate: impl Fn(&AutomationStatus) -> bool,
) -> AutomationStatus {
    timeout(bound, async {
        loop {
            {
                let current = status.borrow_and_update();
                if predicate(&current) {
                    return current.clone();
                }
            }
            status
                .changed()
                .await
                .expect("the mission principal stays alive while awaited on");
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out awaiting {what}"))
}

/// Decodes a broadcast datagram the way every telemetry consumer does,
/// yielding the guidance group when the sample carries one.
fn nav_guidance_of(bytes: &[u8]) -> Option<wire::NavGuidanceState> {
    let envelope = wire::Envelope::decode(bytes).ok()?;
    match envelope.payload? {
        wire::envelope::Payload::TelemetrySample(sample) => sample.nav_guidance.map(|state| *state),
        _ => None,
    }
}

/// Collects guidance samples from broadcast telemetry until `wanted`
/// distinct stamp sequences have been seen, in publication order.
async fn collect_guidance(
    observer: &mut TelemetryObserver,
    wanted: usize,
) -> Vec<wire::NavGuidanceState> {
    timeout(TEST_TIMEOUT, async {
        let mut samples: Vec<wire::NavGuidanceState> = Vec::new();
        while samples.len() < wanted {
            let datagram = observer
                .next_datagram()
                .await
                .expect("the engine actor keeps broadcasting while awaited on");
            let Some(guidance) = nav_guidance_of(&datagram) else {
                continue;
            };
            let sequence = guidance.stamp.as_ref().map(|stamp| stamp.sequence);
            let repeated = samples
                .last()
                .is_some_and(|last| last.stamp.as_ref().map(|s| s.sequence) == sequence);
            // The actor re-publishes the cached group on every tick; only
            // a new mission publication advances the sequence.
            if !repeated {
                samples.push(guidance);
            }
        }
        samples
    })
    .await
    .unwrap_or_else(|_| panic!("timed out awaiting {wanted} guidance publications"))
}

/// Asserts the guidance the actor broadcasts: its own role, a 16-byte
/// incarnation, an advancing sequence, and route-true field values — in
/// the bytes a remote client would receive.
fn assert_guidance_broadcast(guidance: &[wire::NavGuidanceState]) {
    let stamps: Vec<wire::MeasurementStamp> = guidance
        .iter()
        .map(|state| state.stamp.clone().expect("guidance carries its stamp"))
        .collect();
    for stamp in &stamps {
        assert_eq!(
            stamp.role,
            wire::SourceRole::NavigationSolution as i32,
            "guidance travels under its own role, never relabeled"
        );
        assert_eq!(stamp.source_incarnation.len(), 16);
    }
    assert!(
        stamps[1].sequence > stamps[0].sequence,
        "each publication advances the group sequence: {:?}",
        stamps
            .iter()
            .map(|stamp| stamp.sequence)
            .collect::<Vec<_>>()
    );
    for state in guidance {
        assert!(!state.to_ident.is_empty(), "the active waypoint is named");
        assert!(
            DEMO_IDENTS.contains(&state.to_ident.as_str()),
            "the active waypoint comes from the flown route, got {}",
            state.to_ident
        );
        assert!(
            state.distance_to_waypoint_m.is_finite(),
            "distance {} is not a reading",
            state.distance_to_waypoint_m
        );
        assert!(
            (0.0..std::f32::consts::TAU).contains(&state.course_rad),
            "course {} is outside [0, 2π)",
            state.course_rad
        );
        assert_eq!(state.waypoint_count, 3, "the demo route has three fixes");
    }
}

/// Decodes one reliable or authority envelope the way the principal does.
fn envelope_of(bytes: &[u8]) -> Option<wire::Envelope> {
    pilotage_protocol::decode_envelope_length_delimited(bytes)
        .ok()
        .map(|(envelope, _rest)| envelope)
}

/// Opens the operator's session and returns its principal id.
async fn admit(operator: &mut Participant) -> u64 {
    assert!(
        operator
            .send(DomainEnvelope::Hello(ClientHello {
                protocol_version: SESSION_PROTOCOL_VERSION,
                client_name: "operator".to_owned(),
                join_token: Vec::new(),
            }))
            .await
    );
    timeout(TEST_TIMEOUT, async {
        loop {
            let bytes = operator
                .next_bootstrap()
                .await
                .expect("the actor answers the hello");
            if let Some(wire::envelope::Payload::ServerWelcome(welcome)) =
                envelope_of(&bytes).and_then(|envelope| envelope.payload)
            {
                return welcome
                    .principal
                    .expect("the welcome names the principal")
                    .value;
            }
        }
    })
    .await
    .expect("the welcome arrives")
}

/// Awaits the offer of the motion scope to `to`, returning the offering
/// principal.
async fn await_offer(operator: &mut Participant, to: u64) -> u64 {
    timeout(TEST_TIMEOUT, async {
        loop {
            let bytes = operator
                .next_authority()
                .await
                .expect("the actor keeps broadcasting authority events");
            let Some(wire::envelope::Payload::AuthorityEvent(event)) =
                envelope_of(&bytes).and_then(|envelope| envelope.payload)
            else {
                continue;
            };
            if let Some(wire::authority_event::Event::ScopeTransferOffered(offered)) = event.event
                && offered.to_principal.as_ref().is_some_and(|p| p.value == to)
            {
                return offered
                    .from_principal
                    .expect("the offer names the holder")
                    .value;
            }
        }
    })
    .await
    .expect("the holder offers the scope")
}

/// An operator asks for the motion scope while the survey is enroute; the
/// principal offers it and holds. The operator hands it back; the
/// principal accepts, resumes, and completes the survey.
#[tokio::test(start_paused = true)]
async fn an_operator_takes_the_survey_over_and_hands_it_back() {
    let rig = runtime::spawn_reference_survey_rig().expect("survey rig spawns");
    let mut status = rig.status();
    await_status(&mut status, "enroute guidance", |status| {
        status.mission_state == Some(MissionState::Enroute)
    })
    .await;
    let mut operator = rig.participant(77).await;
    let me = admit(&mut operator).await;
    let (vehicle, scope) = (VehicleId::new(1), ScopeId::new("vehicle.motion"));
    assert!(
        operator
            .send(DomainEnvelope::TransferRequest(ScopeTransferRequest {
                vehicle,
                scope: scope.clone(),
            }))
            .await
    );
    let mission_principal = await_offer(&mut operator, me).await;
    assert_ne!(mission_principal, me);
    assert!(
        operator
            .send(DomainEnvelope::TransferAccept(ScopeTransferAccept {
                vehicle,
                scope: scope.clone(),
            }))
            .await
    );
    let held = await_status(&mut status, "the mission to hold", |status| status.held).await;
    assert!(!held.fenced, "a takeover holds, it does not fence");
    assert_eq!(held.lease_generation, None);
    assert_eq!(held.offers, 1);

    assert!(
        operator
            .send(DomainEnvelope::TransferOffer(ScopeTransferOffer {
                vehicle,
                scope,
                to: pilotage_protocol::PrincipalId::new(mission_principal),
            }))
            .await
    );
    // The operator's queue is drained no further: a client the actor
    // cannot reach is evicted on its next send, as a remote connection
    // would be. The offer is queued ahead of that eviction.
    drop(operator);
    let resumed = await_status(&mut status, "the mission to resume", |status| {
        !status.held && status.resumes == 1
    })
    .await;
    assert!(resumed.lease_generation.is_some());
    let complete = await_status_within(
        &mut status,
        Duration::from_secs(600),
        "survey completion after the resume",
        |status| status.mission_state == Some(MissionState::Complete),
    )
    .await;
    assert_eq!(
        complete.frames_rejected, 0,
        "the neutral first frame and every later frame were accepted"
    );
    rig.shutdown().await;
}

/// The survey principal arms, flies every line end of the two-line
/// rectangle in order, and completes; the guidance it broadcasts names
/// the survey's own waypoints.
#[tokio::test(start_paused = true)]
async fn survey_principal_flies_every_line_and_completes() {
    let rig = runtime::spawn_reference_survey_rig().expect("survey rig spawns");
    let mut status = rig.status();
    await_status(&mut status, "arm acceptance", |status| status.arm_accepted).await;
    let mut observer = rig.observe().await;
    let guidance = collect_guidance(&mut observer, 2).await;
    drop(observer);
    for state in &guidance {
        assert_eq!(state.waypoint_count, 4, "two lines, two ends each");
        assert!(
            state.to_ident.starts_with('L'),
            "the active waypoint is a survey line end, got {}",
            state.to_ident
        );
    }
    // Two 60 m lines, a 20 m turn, and the arrivals: a few minutes of
    // virtual time under the paused clock.
    let complete = await_status_within(
        &mut status,
        Duration::from_secs(600),
        "survey completion",
        |status| status.mission_state == Some(MissionState::Complete),
    )
    .await;
    assert_eq!(
        complete.frames_rejected, 0,
        "every streamed frame was accepted"
    );
    assert!(!complete.fenced && !complete.closed);
    rig.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn mission_principal_arms_and_streams_accepted_frames() {
    let rig = runtime::spawn_reference_mission_rig().expect("mission rig spawns");
    let mut status = rig.status();

    // Handshake, activation, then the motion lease, in session order.
    let leased = await_status(&mut status, "the motion lease grant", |status| {
        status.lease_generation.is_some()
    })
    .await;
    assert!(leased.session.is_some(), "the welcome precedes the lease");
    assert!(
        leased.activation_sent,
        "the profile activation precedes the lease"
    );

    // The arm rides the reliable action path and comes back accepted.
    let armed = await_status(&mut status, "arm acceptance", |status| status.arm_accepted).await;
    assert_eq!(
        armed.frames_rejected, 0,
        "no frame was rejected on the way to arming"
    );

    let mut observer = rig.observe().await;
    let guidance = collect_guidance(&mut observer, 2).await;
    assert_guidance_broadcast(&guidance);
    // Eviction is the actor's own path for a departed client.
    drop(observer);

    // With a zero cruise height the mission goes straight enroute and
    // streams a typed intent frame every tick; 200 of them all stay
    // accepted, the lease stays held, and the holder-silence watchdog
    // never fires.
    let streamed = await_status(&mut status, "200 streamed intent frames", |status| {
        status.frames_sent >= 200
    })
    .await;
    assert_eq!(
        streamed.frames_rejected, 0,
        "every streamed frame was accepted: zero FrameRejected"
    );
    assert!(!streamed.fenced, "authority never moved away");
    assert!(!streamed.closed, "the engine never closed the principal");
    assert!(
        streamed.lease_generation.is_some(),
        "the lease is still held after 200 ticks"
    );
    assert!(
        matches!(
            streamed.mission_state,
            Some(MissionState::Enroute | MissionState::Complete)
        ),
        "a zero-cruise mission is enroute (or complete) after arming, got {:?}",
        streamed.mission_state
    );

    // Shutdown drains cleanly: the actor exits when its command channel
    // closes and the principal (holding only a weak handle) follows.
    rig.shutdown().await;
}
