#![allow(clippy::expect_used, clippy::panic)]

//! A live PX4 SITL fixed-wing flight through the unified control contract:
//! arm, the automatic takeoff, a held full turn intent, and a held climb
//! intent, all as typed `Velocity` BODY_FRD frames.
//!
//! Ignored by default. Run it against a running PX4 SITL plane (for example
//! `4003_gz_rc_cessna`) with:
//!
//! ```text
//! PILOTAGE_PX4_LIVE_FW=1 PILOTAGE_PX4_CAMERA=off \
//! PILOTAGE_PX4_ADDR=127.0.0.1:<free port> \
//! PILOTAGE_PX4_GCS_ADDR=127.0.0.1:<18570 + instance> \
//! PILOTAGE_PX4_FC_ADDR=127.0.0.1:<14580 + instance> \
//! PILOTAGE_PX4_SYSTEM_ID=<instance + 1> \
//! cargo test -p pilotage-adapter-px4 --test fixed_wing_sitl -- --ignored --nocapture
//! ```

#[path = "sitl/support.rs"]
mod support;

use std::time::{Duration, Instant};

use pilotage_adapter_api::{Disposition, LinkLossPolicy, VehicleAdapter};
use pilotage_adapter_px4::{FLIGHT_SCOPE, Px4Adapter, Px4Airframe};
use pilotage_protocol::{IntentFamily, ScopeId};
use support::{Observed, Operator, VEHICLE, arm, fly_until, live_config, observe, velocity};
use tracing::info;

/// The advertised flight envelope: (max_vertical, max_angular).
fn advertised_envelope(adapter: &Px4Adapter) -> (f32, f32) {
    let capabilities = adapter.capabilities();
    let intent = capabilities
        .vehicles
        .iter()
        .flat_map(|vehicle| &vehicle.scopes)
        .filter(|scope| scope.scope.as_str() == FLIGHT_SCOPE)
        .flat_map(|scope| &scope.intents)
        .find(|intent| intent.family == IntentFamily::Velocity)
        .expect("the flight scope advertises a velocity intent");
    (intent.effective_vertical(), intent.max_angular)
}

fn unwrap_angle(delta: f64) -> f64 {
    (delta + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// Arms and climbs through 20 m on neutral intent; returns the observation
/// at the arm instant (the takeoff point).
async fn take_off(adapter: &mut Px4Adapter, operator: &mut Operator) -> Observed {
    let armed_after = arm(adapter, operator).await;
    let ground = observe(&adapter.sample_telemetry()).expect("pose at arm");
    info!(
        ?armed_after,
        ground_down_m = ground.down_m,
        "PX4 armed; takeoff sequence running"
    );
    let takeoff_started = Instant::now();
    let (airborne, _) = fly_until(
        adapter,
        operator,
        velocity(0.0, 0.0, 0.0),
        Duration::from_secs(120),
        |observed, _| ground.down_m - observed.down_m >= 20.0,
    )
    .await;
    let airborne = airborne.expect("telemetry during takeoff");
    let takeoff_height = ground.down_m - airborne.down_m;
    info!(takeoff_height, elapsed = ?takeoff_started.elapsed(), "climbed through 20 m");
    assert!(
        takeoff_height >= 20.0,
        "takeoff reached only {takeoff_height} m"
    );
    ground
}

/// Holds a full right turn until the heading has changed by 90°. The
/// intent is constrained (held neutral) until PX4 confirms Position mode,
/// then it is flown.
async fn turn(
    adapter: &mut Px4Adapter,
    operator: &mut Operator,
    ground: Observed,
    max_turn_rate: f32,
) {
    let turn = velocity(0.0, 0.0, max_turn_rate);
    let (handover, disposition) = fly_until(
        adapter,
        operator,
        turn,
        Duration::from_secs(30),
        |_, disposition| *disposition == Disposition::Accepted,
    )
    .await;
    assert_eq!(
        disposition,
        Disposition::Accepted,
        "Position mode never flew the intent"
    );
    let handover = handover.expect("telemetry at handover");
    info!(
        height = ground.down_m - handover.down_m,
        "Position mode flying the turn intent"
    );
    let turn_started = Instant::now();
    let mut previous = handover.heading_rad;
    let mut turned = 0.0_f64;
    fly_until(
        adapter,
        operator,
        turn,
        Duration::from_secs(60),
        |observed, _| {
            turned += unwrap_angle(observed.heading_rad - previous);
            previous = observed.heading_rad;
            turned.abs() >= std::f64::consts::FRAC_PI_2
        },
    )
    .await;
    let turn_time = turn_started.elapsed().as_secs_f64();
    info!(
        turned_deg = turned.to_degrees(),
        turn_time_s = turn_time,
        rate_deg_s = turned.to_degrees() / turn_time,
        commanded_deg_s = f64::from(max_turn_rate).to_degrees(),
        "turn intent held"
    );
    assert!(
        turned >= std::f64::consts::FRAC_PI_2,
        "heading changed only {} deg (+ = clockwise)",
        turned.to_degrees()
    );
}

/// Levels out briefly, then holds a full climb for 15 s.
async fn climb(
    adapter: &mut Px4Adapter,
    operator: &mut Operator,
    ground: Observed,
    max_vertical: f32,
) {
    let (level, _) = fly_until(
        adapter,
        operator,
        velocity(0.0, 0.0, 0.0),
        Duration::from_secs(5),
        |_, _| false,
    )
    .await;
    let climb_start = level.expect("telemetry before the climb");
    let climb_hold = Duration::from_secs(15);
    let (climbed, _) = fly_until(
        adapter,
        operator,
        velocity(0.0, -max_vertical, 0.0),
        climb_hold,
        |_, _| false,
    )
    .await;
    let climbed = climbed.expect("telemetry during the climb");
    let gain = climb_start.down_m - climbed.down_m;
    info!(
        gain_m = gain,
        hold_s = climb_hold.as_secs_f64(),
        rate_mps = gain / climb_hold.as_secs_f32(),
        commanded_mps = max_vertical,
        height = ground.down_m - climbed.down_m,
        "climb intent held"
    );
    assert!(gain >= 10.0, "the climb intent gained only {gain} m");
}

async fn flight() {
    let mut adapter = Px4Adapter::start(VEHICLE, live_config(Px4Airframe::FixedWing))
        .await
        .expect("adapter");
    let mut operator = Operator::default();
    let (max_vertical, max_turn_rate) = advertised_envelope(&adapter);
    info!(
        max_vertical,
        max_turn_rate, "advertised fixed-wing envelope"
    );
    let ground = take_off(&mut adapter, &mut operator).await;
    turn(&mut adapter, &mut operator, ground, max_turn_rate).await;
    climb(&mut adapter, &mut operator, ground, max_vertical).await;
    link_loss_then_resume(&mut adapter, &mut operator, ground, max_turn_rate).await;
}

/// Link loss: the stick stream stops while the GCS heartbeat continues, so
/// PX4's manual-control-loss failsafe alone must hold the plane aloft.
/// Clearing the latch and pressing ARM again resumes Position mode with no
/// second takeoff.
async fn link_loss_then_resume(
    adapter: &mut Px4Adapter,
    operator: &mut Operator,
    ground: Observed,
    max_turn_rate: f32,
) {
    let scope = ScopeId::new(FLIGHT_SCOPE);
    let start = observe(&adapter.sample_telemetry()).expect("telemetry before link loss");
    adapter
        .set_link_loss_policy(VEHICLE, &scope, Some(LinkLossPolicy::Neutralize))
        .expect("neutralize");
    let mut ticker = tokio::time::interval(support::FRAME_PERIOD);
    let hold = Instant::now();
    let (mut lowest, mut highest) = (f32::MAX, f32::MIN);
    let mut armed = true;
    while hold.elapsed() < Duration::from_secs(10) {
        ticker.tick().await;
        let batch = adapter.sample_telemetry();
        if let Some(observed) = observe(&batch) {
            let height = ground.down_m - observed.down_m;
            lowest = lowest.min(height);
            highest = highest.max(height);
        }
        if let Some(state) = batch.samples.iter().find_map(|sample| sample.fc_state) {
            armed = state.arm_state == 2;
        }
    }
    info!(
        start_height = ground.down_m - start.down_m,
        lowest, highest, armed, "10 s of link loss with no stick stream"
    );
    assert!(armed, "the plane disarmed on link loss");
    assert!(lowest >= 20.0, "the plane sank to {lowest} m on link loss");

    adapter
        .set_link_loss_policy(VEHICLE, &scope, None)
        .expect("clear");
    arm(adapter, operator).await;
    let turn = velocity(0.0, 0.0, max_turn_rate);
    let (_, disposition) = fly_until(
        adapter,
        operator,
        turn,
        Duration::from_secs(10),
        |_, disposition| *disposition == Disposition::Accepted,
    )
    .await;
    info!(?disposition, "re-armed after link loss");
    assert_eq!(
        disposition,
        Disposition::Accepted,
        "Position mode did not resume"
    );
}

#[test]
#[ignore = "flies a live PX4 SITL plane; see the module docs"]
fn a_px4_plane_takes_off_turns_and_climbs_on_the_unified_intent() {
    if std::env::var("PILOTAGE_PX4_LIVE_FW").as_deref() != Ok("1") {
        panic!("set PILOTAGE_PX4_LIVE_FW=1 and the PX4 endpoints to run this flight");
    }
    support::run(flight());
}
