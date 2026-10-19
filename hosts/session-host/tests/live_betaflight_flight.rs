//! A Betaflight SITL quad flown by a Pilotage client through the real host.
//!
//! Ignored by default. It needs `betaflight_SITL.elf` provisioned with
//! `sim/betaflight/sitl.cli` and `pilotage-multirotor-fdm` closing its
//! loop (`cargo xtask sim betaflight-sitl` brings both up), then:
//!
//! ```text
//! PILOTAGE_BETAFLIGHT_SITL=1 PILOTAGE_BETAFLIGHT_PROFILE=simulation \
//!     cargo test -p pilotage-session-host --test live_betaflight_flight \
//!     -- --ignored --nocapture
//! ```
//!
//! With `PILOTAGE_LIVE_HOST_PORT=<port>` it flies the vehicle of a session
//! host already listening on that loopback port instead, and with
//! `PILOTAGE_LIVE_SKIP_DIRECT=1` it leaves out the direct-scope step, for
//! a vehicle whose adapter does not hand the motion lease to the direct
//! scope in flight.
//!
//! The client is the portable client-session core, as every Pilotage
//! client is: it is admitted, takes the motion lease, arms through the
//! reliable action path, and sends typed velocity intents scaled by the
//! envelope the host advertises. The flight: arm, climb at least 2 m, hold
//! within 1 m for 5 s at neutral; lease the direct scope and fly a 1 rad/s
//! roll-rate step (measured within 0.3 rad/s), lease the velocity scope
//! back and hold level; at least 5 m forward, at least 45 degrees of yaw,
//! land, disarm.

#![allow(clippy::expect_used, clippy::panic)]

#[path = "live_flight/client.rs"]
mod client;

use std::time::{Duration, Instant};

use client::{Client, Reading};
use pilotage_protocol::wire;
use pilotage_session_host::cli::AdapterKind;
use pilotage_session_host::runtime;

const CONTROL_ARM: u32 = 1;
const CONTROL_DISARM: u32 = 2;

fn heading_change(from: f64, to: f64) -> f64 {
    (to - from).to_degrees().rem_euclid(360.0)
}

#[test]
#[ignore = "needs Betaflight SITL and pilotage-multirotor-fdm; set PILOTAGE_BETAFLIGHT_SITL=1"]
fn a_client_flies_a_betaflight_quad_through_the_host() {
    if std::env::var("PILOTAGE_BETAFLIGHT_SITL").as_deref() != Ok("1") {
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
        .block_on(flight());
}

async fn flight() {
    // PILOTAGE_LIVE_HOST_PORT flies the vehicle of a host already running
    // (a `cargo xtask sim --fc betaflight-sitl` session) instead of one
    // this test starts.
    let external = std::env::var("PILOTAGE_LIVE_HOST_PORT").ok().map(|port| {
        port.parse::<u16>()
            .expect("PILOTAGE_LIVE_HOST_PORT is a port")
    });
    let host = match external {
        Some(_) => None,
        None => Some(
            runtime::start_with_options(
                0,
                AdapterKind::Betaflight,
                runtime::RuntimeOptions::default(),
            )
            .await
            .expect("host starts (PILOTAGE_BETAFLIGHT_PROFILE=simulation)"),
        ),
    };
    let addr = host.as_ref().map_or_else(
        || std::net::SocketAddr::from(([127, 0, 0, 1], external.unwrap_or_default())),
        |host| host.local_addr,
    );
    let mut client = Client::connect(addr).await;
    client.admit_and_lease().await;
    let first = client
        .fly([0.0; 4], Duration::from_secs(30), |_| true)
        .await;
    tracing::info!(?first, "telemetry through the host");
    arm(&mut client).await;
    climb(&mut client).await;
    hold(&mut client).await;
    if std::env::var("PILOTAGE_LIVE_SKIP_DIRECT").as_deref() != Ok("1") {
        direct(&mut client).await;
    }
    forward(&mut client).await;
    yaw(&mut client).await;
    land_and_disarm(&mut client).await;
    if let Some(host) = host {
        host.shutdown().await;
    }
}

/// Arms, retrying while Betaflight names a blocker (it reports RXLOSS and
/// BOOTGRACE for its first seconds).
async fn arm(client: &mut Client) {
    for _ in 0..20 {
        let verdict = client.action(CONTROL_ARM).await;
        tracing::info!(?verdict, "arm");
        if client
            .fly([0.0; 4], Duration::from_secs(3), |r| r.armed)
            .await
            .armed
        {
            return;
        }
    }
    panic!("Betaflight did not arm");
}

/// Throttle 2/3 of the 1.5 m/s envelope: a 1 m/s climb.
async fn climb(client: &mut Client) {
    // Betaflight zeroes its altitude at the arm transition; read the
    // ground level after it has.
    let level = client
        .fly([0.0; 4], Duration::from_secs(1), |_| false)
        .await;
    let climbed = client
        .fly([0.0, 0.0, 2.0 / 3.0, 0.0], Duration::from_secs(10), |r| {
            r.up - level.up >= 3.0
        })
        .await;
    let climb = climbed.up - level.up;
    tracing::info!(ground = level.up, climb, "climbed");
    assert!(climb >= 2.0, "climbed only {climb:.2} m");
}

async fn hold(client: &mut Client) {
    client
        .fly([0.0; 4], Duration::from_secs(4), |_| false)
        .await;
    let anchor = client.fly([0.0; 4], Duration::ZERO, |_| true).await;
    let (mut worst, mut worst_up) = (0.0f64, 0.0f64);
    client
        .fly([0.0; 4], Duration::from_secs(5), |r| {
            worst = worst.max((r.north - anchor.north).hypot(r.east - anchor.east));
            worst_up = worst_up.max((r.up - anchor.up).abs());
            false
        })
        .await;
    tracing::info!(
        horizontal_m = worst,
        vertical_m = worst_up,
        "held at neutral for 5 s"
    );
    assert!(
        worst <= 1.0 && worst_up <= 1.0,
        "drifted {worst:.2} m / {worst_up:.2} m"
    );
}

fn body_rate(roll_rate: f32) -> wire::ControlIntent {
    wire::ControlIntent {
        family: Some(wire::control_intent::Family::BodyRate(
            wire::BodyRateIntent {
                roll_rate,
                pitch_rate: 0.0,
                yaw_rate: 0.0,
                // Near hover, so the operator's thrust takes over at once.
                thrust: 0.57,
            },
        )),
    }
}

/// The racing scheme: a body-rate step on the direct scope, then back.
async fn direct(client: &mut Client) {
    client.switch_to("vehicle.motion.direct").await;
    client
        .fly_intent(&body_rate(0.0), Duration::from_millis(500), |_| false)
        .await;
    let start = Instant::now();
    let mut samples = Vec::new();
    client
        .fly_intent(&body_rate(1.0), Duration::from_millis(400), |r| {
            if start.elapsed() >= Duration::from_millis(200) {
                samples.push(r.roll_rate);
            }
            false
        })
        .await;
    client
        .fly_intent(&body_rate(-1.0), Duration::from_millis(400), |_| false)
        .await;
    client
        .fly_intent(&body_rate(0.0), Duration::from_millis(300), |_| false)
        .await;
    samples.sort_by(f64::total_cmp);
    let measured = samples.get(samples.len() / 2).copied().unwrap_or(f64::NAN);
    tracing::info!(roll_rate = measured, "body rate for a 1 rad/s demand");
    assert!((measured - 1.0).abs() <= 0.3, "roll rate {measured:.2}");
    client.switch_to("vehicle.motion").await;
    let settled = client
        .fly([0.0; 4], Duration::from_secs(4), |_| false)
        .await;
    tracing::info!(
        up = settled.up,
        tilt_deg = settled.tilt_deg,
        "assisted again"
    );
    assert!(settled.up > 1.0 && settled.tilt_deg < 5.0, "{settled:?}");
}

/// Pitch 2/3 of the 3 m/s envelope: 2 m/s forward.
async fn forward(client: &mut Client) {
    let start = client.fly([0.0; 4], Duration::ZERO, |_| true).await;
    let (sin, cos) = start.heading.sin_cos();
    let along = |r: &Reading| (r.north - start.north) * cos + (r.east - start.east) * sin;
    let moved = client
        .fly([0.0, 2.0 / 3.0, 0.0, 0.0], Duration::from_secs(8), |r| {
            along(r) >= 8.0
        })
        .await;
    tracing::info!(forward_m = along(&moved), "flew forward");
    assert!(along(&moved) >= 5.0, "forward only {:.2} m", along(&moved));
}

/// Yaw 2/3 of the 1.5 rad/s envelope: 1 rad/s clockwise.
async fn yaw(client: &mut Client) {
    client
        .fly([0.0; 4], Duration::from_secs(4), |_| false)
        .await;
    let before = client.fly([0.0; 4], Duration::ZERO, |_| true).await;
    let yawed = client
        .fly([0.0, 0.0, 0.0, 2.0 / 3.0], Duration::from_secs(3), |r| {
            (60.0..180.0).contains(&heading_change(before.heading, r.heading))
        })
        .await;
    let turned = heading_change(before.heading, yawed.heading);
    tracing::info!(turned_deg = turned, "yawed");
    assert!((45.0..180.0).contains(&turned), "yawed {turned:.1} deg");
}

async fn land_and_disarm(client: &mut Client) {
    client
        .fly([0.0; 4], Duration::from_secs(2), |_| false)
        .await;
    let landed = client
        .fly([0.0, 0.0, -2.0 / 3.0, 0.0], Duration::from_secs(20), |r| {
            r.up < 0.15
        })
        .await;
    client
        .fly([0.0, 0.0, -2.0 / 3.0, 0.0], Duration::from_secs(2), |_| {
            false
        })
        .await;
    tracing::info!(up = landed.up, "landed");
    // An FC with a land detector (PX4) refuses a disarm until it agrees
    // the vehicle is down, which can take it ten seconds under a held
    // descent demand, so the disarm is asked again while it refuses.
    for _ in 0..10 {
        let verdict = client.action(CONTROL_DISARM).await;
        tracing::info!(?verdict, "disarm");
        if !client
            .fly([0.0; 4], Duration::from_secs(3), |r| !r.armed)
            .await
            .armed
        {
            return;
        }
    }
    panic!("the vehicle did not disarm");
}
