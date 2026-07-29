//! In-process mission-executor integration test: the reference engine
//! actor and the mission principal wired exactly as the runtime wires
//! them for the reference adapter — no transport endpoint, no network.
//!
//! The planar skiff cannot traverse geodetic waypoints meaningfully, so
//! the test scopes to the fenced in-process path itself: the principal
//! completes the handshake, announces its activation, leases the motion
//! scope, arms over the reliable action path, then streams typed intent
//! frames for 200 ticks with ZERO `FrameRejected` and the lease still
//! held. Every wait is event-driven on the status watch under a bounded
//! timeout — never a sleep-and-poll.

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_mission::MissionState;
use pilotage_session_host::runtime::{self, AutomationStatus};
use tokio::time::timeout;

/// Virtual-time bound (the test runs under a paused clock, so this never
/// costs wall time); a stalled rig fails fast at this deadline.
const TEST_TIMEOUT: Duration = Duration::from_secs(120);

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
