//! A PX4 SITL gimbal, driven by a Pilotage client through the real host,
//! moves at the rate the `vehicle.gimbal` scope demands.
//!
//! Ignored by default. It needs a running PX4 gimbal session whose host
//! listens on a loopback port (for example
//! `cargo xtask fleet --vehicle px4-jsbsim:quad --port 4633`) and no other
//! client holding the gimbal, then:
//!
//! ```text
//! PILOTAGE_PX4_LIVE_GIMBAL=1 PILOTAGE_LIVE_HOST_PORT=4633 \
//!     cargo test -p pilotage-session-host --test live_px4_gimbal_rate \
//!     -- --ignored --nocapture
//! ```
//!
//! The client leases the gimbal scope, recenters, streams a pitch-rate
//! demand of half the advertised envelope for 2 s, and fits the slope of
//! the pitch the vehicle reports (GIMBAL_DEVICE_ATTITUDE_STATUS). The
//! vehicle stays disarmed; PX4's gimbal module runs and reports regardless.

#![allow(clippy::expect_used, clippy::panic)]

#[path = "live_flight/client.rs"]
mod client;

use std::time::{Duration, Instant};

use client::Client;
use pilotage_client_session::{gimbal_rate_intent, intent_capability};
use pilotage_protocol::wire;

const GIMBAL_SCOPE: &str = "vehicle.gimbal";
/// Half the advertised envelope: the demand does not clamp and the gimbal
/// stays inside its landed pitch limit within the window.
const PITCH_FRACTION: f32 = -0.5;
/// How long the demand streams.
const STREAM: Duration = Duration::from_secs(2);
/// Samples before this are the gimbal starting to move.
const SETTLE: Duration = Duration::from_millis(400);

#[test]
#[ignore = "needs a running PX4 gimbal session; set PILOTAGE_PX4_LIVE_GIMBAL=1 and PILOTAGE_LIVE_HOST_PORT"]
fn a_gimbal_moves_at_the_rate_the_operator_demands() {
    if std::env::var("PILOTAGE_PX4_LIVE_GIMBAL").as_deref() != Ok("1") {
        return;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init()
        .ok();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(measure());
}

/// Least-squares slope of `(seconds, value)` samples.
fn slope(samples: &[(f64, f64)]) -> f64 {
    let n = samples.len() as f64;
    let mx = samples.iter().map(|(x, _)| x).sum::<f64>() / n;
    let my = samples.iter().map(|(_, y)| y).sum::<f64>() / n;
    let num: f64 = samples.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
    let den: f64 = samples.iter().map(|(x, _)| (x - mx) * (x - mx)).sum();
    num / den
}

fn gimbal_demand(client: &Client, pitch: f32) -> (wire::ControlIntent, f32) {
    let admission = client.admission();
    let vehicle = admission.vehicles[0].vehicle_id;
    let capability = intent_capability(
        admission,
        vehicle,
        GIMBAL_SCOPE,
        wire::IntentFamily::GimbalRate,
    )
    .expect("the vehicle advertises a gimbal rate envelope");
    let intent = gimbal_rate_intent(pitch, 0.0, Some(capability)).expect("a gimbal-rate intent");
    (intent, pitch * capability.max_angular)
}

async fn measure() {
    let port: u16 = std::env::var("PILOTAGE_LIVE_HOST_PORT")
        .expect("PILOTAGE_LIVE_HOST_PORT names the session host")
        .parse()
        .expect("PILOTAGE_LIVE_HOST_PORT is a port");
    let mut client = Client::connect(std::net::SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.admit_and_lease().await;
    client.switch_to(GIMBAL_SCOPE).await;

    let (idle, _) = gimbal_demand(&client, 0.0);
    // A neutral frame from the new holder clears a link-loss latch a
    // previous holder's disconnect left on the scope.
    client
        .fly_intent(&idle, Duration::from_secs(1), |_| false)
        .await;
    client.recenter().await;
    client
        .fly_intent(&idle, Duration::from_secs(3), |_| false)
        .await;

    let (demand, commanded) = gimbal_demand(&client, PITCH_FRACTION);
    let started = Instant::now();
    let mut samples = Vec::new();
    client
        .fly_intent(&demand, STREAM, |reading| {
            let at = started.elapsed();
            if at >= SETTLE
                && let Some(pitch) = reading.gimbal_pitch
            {
                samples.push((at.as_secs_f64(), pitch));
            }
            false
        })
        .await;
    client
        .fly_intent(&idle, Duration::from_millis(300), |_| false)
        .await;

    assert!(
        samples.len() >= 10,
        "too few gimbal reports: {}",
        samples.len()
    );
    let observed = slope(&samples);
    let ratio = observed / f64::from(commanded);
    tracing::info!(
        commanded_rps = commanded,
        observed_rps = observed,
        ratio,
        samples = samples.len(),
        "gimbal pitch rate"
    );
    assert!(
        (0.7..=1.3).contains(&ratio),
        "the gimbal moved at {observed:.4} rad/s for a {commanded} rad/s demand ({ratio:.3}x)"
    );
}
