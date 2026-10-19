#![allow(clippy::expect_used, clippy::panic)]

//! A live PX4 SITL multicopter flight through both flight families: climb
//! on `Velocity`, switch in the hover to `BodyRate` on the direct scope,
//! fly a roll-rate step, hold an `AttitudeThrust` bank, and switch back.
//!
//! Ignored by default. Run it against a running PX4 SITL quad (for example
//! `4001_gz_x500`) with the same environment as `fixed_wing_sitl`, but
//! `PILOTAGE_PX4_LIVE_MC=1`:
//!
//! ```text
//! PILOTAGE_PX4_LIVE_MC=1 PILOTAGE_PX4_CAMERA=off \
//! PILOTAGE_PX4_ADDR=127.0.0.1:<free port> \
//! PILOTAGE_PX4_GCS_ADDR=127.0.0.1:<18570 + instance> \
//! PILOTAGE_PX4_FC_ADDR=127.0.0.1:<14580 + instance> \
//! PILOTAGE_PX4_SYSTEM_ID=<instance + 1> \
//! cargo test -p pilotage-adapter-px4 --test multicopter_direct_sitl -- --ignored --nocapture
//! ```

#[path = "sitl/support.rs"]
mod support;

use std::time::{Duration, Instant};

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_adapter_px4::{DIRECT_SCOPE, Px4Adapter, Px4Airframe};
use pilotage_protocol::{AttitudeThrustIntent, BodyRateIntent, ControlIntent, ReferenceFrame};
use support::{
    Demand, Observed, Operator, VEHICLE, arm, fly_until, live_config, observe, velocity,
};
use tracing::info;

/// A far-from-hover operator thrust: the adapter must hold PX4's own thrust
/// instead of flying it.
const LOW_THRUST: f32 = 0.2;

fn body_rate(roll_rate: f32) -> Demand {
    Demand {
        scope: DIRECT_SCOPE,
        intent: ControlIntent::BodyRate(BodyRateIntent {
            roll_rate,
            pitch_rate: 0.0,
            yaw_rate: 0.0,
            thrust: LOW_THRUST,
        }),
    }
}

fn bank(roll_rad: f32, yaw_rad: f32) -> Demand {
    let (sr, cr) = (roll_rad * 0.5).sin_cos();
    let (sy, cy) = (yaw_rad * 0.5).sin_cos();
    Demand {
        scope: DIRECT_SCOPE,
        intent: ControlIntent::AttitudeThrust(AttitudeThrustIntent {
            frame: ReferenceFrame::LocalNed,
            qw: cr * cy,
            qx: sr * cy,
            qy: sr * sy,
            qz: cr * sy,
            thrust: LOW_THRUST,
        }),
    }
}

fn roll_of(observed: Observed) -> f32 {
    let [w, x, y, z] = observed.quat_wxyz;
    (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y))
}

/// Holds `demand` for `hold` and returns every observation.
async fn hold(
    adapter: &mut Px4Adapter,
    operator: &mut Operator,
    demand: Demand,
    hold: Duration,
) -> (Vec<(Duration, Observed)>, Disposition) {
    let started = Instant::now();
    let mut seen = Vec::new();
    let (_, disposition) = fly_until(adapter, operator, demand, hold, |observed, _| {
        seen.push((started.elapsed(), observed));
        false
    })
    .await;
    (seen, disposition)
}

/// Arms, climbs to 10 m, and settles in a velocity hover; returns the
/// takeoff point.
async fn climb_to_hover(adapter: &mut Px4Adapter, operator: &mut Operator) -> Observed {
    arm(adapter, operator).await;
    let ground = observe(&adapter.sample_telemetry()).expect("pose at arm");
    let (top, _) = fly_until(
        adapter,
        operator,
        velocity(0.0, -1.5, 0.0),
        Duration::from_secs(30),
        |observed, _| ground.down_m - observed.down_m >= 10.0,
    )
    .await;
    let top = top.expect("telemetry in the climb");
    info!(height = ground.down_m - top.down_m, "climbed on Velocity");
    hold(
        adapter,
        operator,
        velocity(0.0, 0.0, 0.0),
        Duration::from_secs(4),
    )
    .await;
    ground
}

/// The switch into direct flight in the hover, with an operator thrust far
/// below hover: the adapter holds PX4's thrust, so the height holds.
async fn switch_to_body_rate(adapter: &mut Px4Adapter, operator: &mut Operator, ground: Observed) {
    let before = observe(&adapter.sample_telemetry()).expect("pose before the switch");
    let (seen, disposition) = hold(adapter, operator, body_rate(0.0), Duration::from_secs(3)).await;
    assert_eq!(
        disposition,
        Disposition::Constrained,
        "thrust held for takeover"
    );
    let lowest = seen.iter().map(|(_, o)| o.down_m).fold(f32::MIN, f32::max);
    let loss = lowest - before.down_m;
    info!(
        height = ground.down_m - before.down_m,
        loss_m = loss,
        "3 s on BodyRate (zero rates, operator thrust 0.2) after the switch"
    );
    assert!(loss < 1.0, "the family switch lost {loss} m");
}

/// A roll-rate step, then the opposite step back toward level. Returns the
/// mean measured roll rate in each step after a 0.15 s rise.
async fn roll_rate_steps(adapter: &mut Px4Adapter, operator: &mut Operator) -> [f32; 2] {
    let mut means = [0.0; 2];
    for (mean, commanded) in means.iter_mut().zip([1.0_f32, -1.0]) {
        let (seen, _) = hold(
            adapter,
            operator,
            body_rate(commanded),
            Duration::from_millis(400),
        )
        .await;
        let settled: Vec<f32> = seen
            .iter()
            .filter(|(at, _)| *at >= Duration::from_millis(150))
            .map(|(_, o)| o.rates_rps[0])
            .collect();
        *mean = settled.iter().sum::<f32>() / settled.len().max(1) as f32;
        info!(
            commanded,
            measured = *mean,
            samples = settled.len(),
            "roll-rate step"
        );
    }
    hold(
        adapter,
        operator,
        body_rate(0.0),
        Duration::from_millis(500),
    )
    .await;
    means
}

async fn flight() {
    let mut adapter = Px4Adapter::start(VEHICLE, live_config(Px4Airframe::Multicopter))
        .await
        .expect("adapter");
    let mut operator = Operator::default();
    let ground = climb_to_hover(&mut adapter, &mut operator).await;
    switch_to_body_rate(&mut adapter, &mut operator, ground).await;

    let [up, down] = roll_rate_steps(&mut adapter, &mut operator).await;
    assert!(
        (up - 1.0).abs() <= 0.3,
        "commanded 1.0 rad/s, measured {up}"
    );
    assert!(
        (down + 1.0).abs() <= 0.3,
        "commanded -1.0 rad/s, measured {down}"
    );

    let heading = observe(&adapter.sample_telemetry())
        .expect("pose")
        .heading_rad as f32;
    let target = 10.0_f32.to_radians();
    let (seen, _) = hold(
        &mut adapter,
        &mut operator,
        bank(target, heading),
        Duration::from_millis(1500),
    )
    .await;
    let reached = seen
        .last()
        .map(|(_, o)| roll_of(*o))
        .expect("telemetry in the bank");
    info!(
        commanded_deg = 10.0,
        measured_deg = reached.to_degrees(),
        "AttitudeThrust bank"
    );
    assert!(
        (reached - target).abs() <= 3.0_f32.to_radians(),
        "bank reached {reached} rad"
    );

    let at_switch = observe(&adapter.sample_telemetry()).expect("pose before switching back");
    let (seen, disposition) = hold(
        &mut adapter,
        &mut operator,
        velocity(0.0, 0.0, 0.0),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(disposition, Disposition::Accepted);
    let lowest = seen.iter().map(|(_, o)| o.down_m).fold(f32::MIN, f32::max);
    let level = seen
        .last()
        .map(|(_, o)| roll_of(*o))
        .expect("telemetry after the switch");
    info!(
        loss_m = lowest - at_switch.down_m,
        roll_deg = level.to_degrees(),
        height = ground.down_m - seen.last().map_or(at_switch.down_m, |(_, o)| o.down_m),
        "5 s back on Velocity"
    );
    assert!(
        level.abs() <= 3.0_f32.to_radians(),
        "not level again: {level} rad"
    );
}

#[test]
#[ignore = "flies a live PX4 SITL multicopter; see the module docs"]
fn a_px4_quad_switches_between_velocity_and_direct_flight_in_the_hover() {
    if std::env::var("PILOTAGE_PX4_LIVE_MC").as_deref() != Ok("1") {
        panic!("set PILOTAGE_PX4_LIVE_MC=1 and the PX4 endpoints to run this flight");
    }
    support::run(flight());
}
