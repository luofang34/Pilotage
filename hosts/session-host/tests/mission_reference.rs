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

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_mission::MissionState;
use pilotage_protocol::wire;
use pilotage_session_host::runtime::{self, AutomationStatus, TelemetryObserver};
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
    timeout(TEST_TIMEOUT, async {
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
