//! Live flight of a Betaflight SITL quad through the public adapter API.
//!
//! Ignored by default. It needs `betaflight_SITL.elf` provisioned with
//! `sim/betaflight/sitl.cli` and `pilotage-multirotor-fdm` closing its
//! loop (as `cargo xtask sim betaflight-sitl` launches them), then:
//!
//! ```text
//! PILOTAGE_BETAFLIGHT_SITL=1 cargo test -p pilotage-adapter-betaflight \
//!     --test live_sitl_flight -- --ignored --nocapture
//! ```
//!
//! The flight: arm, climb at least 2 m on a climb intent, hold position
//! within 1 m for 5 s at neutral; then direct flight: body-rate steps in
//! ACRO within 0.3 rad/s (roll) and 0.4 rad/s (yaw) of the demand, a 10 deg
//! roll in ANGLE within 3 deg; back to assisted flight holding level; then
//! at least 5 m forward on a forward intent, at least 45 degrees on a yaw
//! intent, land, and disarm.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use pilotage_adapter_api::{TelemetrySample, VehicleAdapter};
use pilotage_adapter_betaflight::{
    BetaflightAdapter, BetaflightConfig, BetaflightProfile, DIRECT_SCOPE, FLIGHT_SCOPE,
};
use pilotage_protocol::{
    AttitudeThrustIntent, BodyRateIntent, ControlAction, ControlIntent, ControlPayload, Generation,
    ReferenceFrame, ScopeId, ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;

/// Control frames go out at the rate a client sends them: a camera-drone
/// client at 20 Hz, a racing client at 100 Hz.
const FRAME_PERIOD: Duration = Duration::from_millis(50);
const DIRECT_FRAME_PERIOD: Duration = Duration::from_millis(10);

fn frame(sequence: u32, velocity: [f32; 4], actions: Vec<ControlAction>) -> ScopedControlFrame {
    let intent = ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: velocity[0],
        vy: velocity[1],
        vz: velocity[2],
        yaw_rate: velocity[3],
    });
    scoped(FLIGHT_SCOPE, sequence, intent, actions)
}

fn scoped(
    scope: &str,
    sequence: u32,
    intent: ControlIntent,
    actions: Vec<ControlAction>,
) -> ScopedControlFrame {
    ScopedControlFrame {
        timing: None,
        admitted_timing: None,
        action_ids: vec![],
        session: SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new(scope),
        generation: Generation::new(1),
        sequence: SequenceNum::new(sequence),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent: Some(intent),
        actions,
    }
}

/// What the test reads from one telemetry sample.
#[derive(Debug, Clone, Copy)]
struct Reading {
    armed: bool,
    north: f64,
    east: f64,
    up: f64,
    heading: f64,
    tilt_deg: f64,
    roll_deg: f64,
    /// Body rates p, q, r, rad/s.
    rates: [f64; 3],
}

fn read(sample: &TelemetrySample) -> Option<Reading> {
    let avionics = sample.avionics?;
    let kinematics = avionics.kinematics?;
    let pose = sample.pose?;
    let attitude = avionics.attitude?;
    let [w, x, y, z] = attitude.quat_wxyz.map(f64::from);
    // The angle between body down and world down.
    let tilt_deg = (1.0 - 2.0 * (x * x + y * y))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
    Some(Reading {
        armed: sample.fc_state?.arm_state == 2,
        north: f64::from(kinematics.pos_ned_m[0]),
        east: f64::from(kinematics.pos_ned_m[1]),
        up: -f64::from(kinematics.pos_ned_m[2]),
        heading: pose.heading,
        tilt_deg,
        roll_deg: (2.0 * (w * x + y * z))
            .atan2(1.0 - 2.0 * (x * x + y * y))
            .to_degrees(),
        rates: attitude.rates_rps.map(f64::from),
    })
}

struct Pilot {
    adapter: BetaflightAdapter,
    sequence: u32,
}

impl Pilot {
    /// Flies `velocity` (vx, vy, vz, yaw rate) until `done` holds or
    /// `limit` passes; returns the last reading.
    async fn fly(
        &mut self,
        velocity: [f32; 4],
        limit: Duration,
        done: impl FnMut(&Reading) -> bool,
    ) -> Reading {
        let [vx, vy, vz, yaw_rate] = velocity;
        let intent = ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx,
            vy,
            vz,
            yaw_rate,
        });
        self.fly_intent(FLIGHT_SCOPE, intent, limit, done).await
    }

    /// Flies a direct intent on the direct scope at a racing client's rate.
    async fn fly_direct(
        &mut self,
        intent: ControlIntent,
        limit: Duration,
        done: impl FnMut(&Reading) -> bool,
    ) -> Reading {
        self.fly_intent(DIRECT_SCOPE, intent, limit, done).await
    }

    async fn fly_intent(
        &mut self,
        scope: &str,
        intent: ControlIntent,
        limit: Duration,
        mut done: impl FnMut(&Reading) -> bool,
    ) -> Reading {
        let start = Instant::now();
        let period = if scope == DIRECT_SCOPE {
            DIRECT_FRAME_PERIOD
        } else {
            FRAME_PERIOD
        };
        let mut ticker = tokio::time::interval(period);
        loop {
            ticker.tick().await;
            self.sequence = self.sequence.wrapping_add(1);
            self.adapter
                .apply_control(&scoped(scope, self.sequence, intent, vec![]));
            let reading = self.reading();
            if let Some(reading) = reading {
                assert!(
                    reading.tilt_deg < 45.0,
                    "the quad tilted {:.0} deg: {reading:?}",
                    reading.tilt_deg
                );
            }
            if let Some(reading) = reading
                && (done(&reading) || start.elapsed() >= limit)
            {
                return reading;
            }
            assert!(
                start.elapsed() < limit + Duration::from_secs(5),
                "telemetry lost"
            );
        }
    }

    fn reading(&mut self) -> Option<Reading> {
        self.adapter
            .sample_telemetry()
            .samples
            .first()
            .and_then(read)
    }

    async fn action(&mut self, action: ControlAction) -> String {
        self.sequence = self.sequence.wrapping_add(1);
        let outcome = self
            .adapter
            .apply_control(&frame(self.sequence, [0.0; 4], vec![action]));
        outcome
            .action_results
            .first()
            .map_or_else(String::new, |result| {
                format!("accepted={} {}", result.accepted, result.detail)
            })
    }
}

fn heading_change(from: f64, to: f64) -> f64 {
    let mut delta = (to - from).to_degrees();
    while delta > 180.0 {
        delta -= 360.0;
    }
    while delta < -180.0 {
        delta += 360.0;
    }
    delta
}

#[test]
#[ignore = "needs Betaflight SITL and pilotage-multirotor-fdm; set PILOTAGE_BETAFLIGHT_SITL=1"]
fn a_betaflight_quad_flies_assisted_and_direct() {
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
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(flight());
}

async fn flight() {
    let config = BetaflightConfig::new(BetaflightProfile::Simulation);
    let mut pilot = Pilot {
        adapter: BetaflightAdapter::start(VehicleId::new(1), config),
        sequence: 0,
    };
    let first = pilot.fly([0.0; 4], Duration::from_secs(30), |_| true).await;
    tracing::info!(?first, "telemetry with a fix");
    arm(&mut pilot).await;
    climb(&mut pilot).await;
    hold(&mut pilot).await;
    direct_rates(&mut pilot).await;
    direct_attitude(&mut pilot).await;
    back_to_assisted(&mut pilot).await;
    forward(&mut pilot).await;
    yaw(&mut pilot).await;
    land_and_disarm(&mut pilot).await;
}

/// Arms, retrying while Betaflight names a blocker (it reports RXLOSS and
/// BOOTGRACE for its first seconds).
async fn arm(pilot: &mut Pilot) {
    for _ in 0..40 {
        let verdict = pilot.action(ControlAction::Arm).await;
        tracing::info!(%verdict, "arm");
        if pilot
            .fly([0.0; 4], Duration::from_secs(3), |r| r.armed)
            .await
            .armed
        {
            return;
        }
    }
    panic!("Betaflight did not arm");
}

async fn climb(pilot: &mut Pilot) {
    // Betaflight zeroes its altitude at the arm transition; read the
    // ground level after it has.
    let level = pilot.fly([0.0; 4], Duration::from_secs(1), |_| false).await;
    let climbed = pilot
        .fly([0.0, 0.0, -1.0, 0.0], Duration::from_secs(10), |r| {
            r.up - level.up >= 3.0
        })
        .await;
    let climb = climbed.up - level.up;
    tracing::info!(ground = level.up, climb, "climbed on a climb intent");
    assert!(climb >= 2.0, "climbed only {climb:.2} m");
}

async fn hold(pilot: &mut Pilot) {
    pilot.fly([0.0; 4], Duration::from_secs(4), |_| false).await;
    let anchor = pilot.fly([0.0; 4], Duration::ZERO, |_| true).await;
    let mut worst = 0.0f64;
    let mut worst_up = 0.0f64;
    pilot
        .fly([0.0; 4], Duration::from_secs(5), |r| {
            worst = worst.max((r.north - anchor.north).hypot(r.east - anchor.east));
            worst_up = worst_up.max((r.up - anchor.up).abs());
            false
        })
        .await;
    tracing::info!(
        horizontal_m = worst,
        vertical_m = worst_up,
        "largest excursion over 5 s at neutral"
    );
    assert!(worst <= 1.0, "drifted {worst:.2} m while holding");
    assert!(
        worst_up <= 1.0,
        "altitude moved {worst_up:.2} m while holding"
    );
}

/// Near the hover throttle the operator's thrust takes over at once.
const HOVER_THRUST: f32 = 0.57;

fn rates(roll_rate: f32, yaw_rate: f32) -> ControlIntent {
    ControlIntent::BodyRate(BodyRateIntent {
        roll_rate,
        pitch_rate: 0.0,
        yaw_rate,
        thrust: HOVER_THRUST,
    })
}

/// The median of `samples`.
fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples.get(samples.len() / 2).copied().unwrap_or(f64::NAN)
}

/// Commands `intent` for `seconds` and returns the median body rate on
/// `axis` over the second half, once the step has settled.
async fn measured_rate(pilot: &mut Pilot, intent: ControlIntent, seconds: f64, axis: usize) -> f64 {
    let start = Instant::now();
    let mut samples = Vec::new();
    pilot
        .fly_direct(intent, Duration::from_secs_f64(seconds), |r| {
            if start.elapsed().as_secs_f64() >= seconds / 2.0 {
                samples.push(r.rates[axis]);
            }
            false
        })
        .await;
    median(samples)
}

/// Direct flight in ACRO: a body-rate step produces that body rate.
async fn direct_rates(pilot: &mut Pilot) {
    pilot
        .fly_direct(rates(0.0, 0.0), Duration::from_secs(1), |_| false)
        .await;
    let right = measured_rate(pilot, rates(1.0, 0.0), 0.4, 0).await;
    let left = measured_rate(pilot, rates(-1.0, 0.0), 0.4, 0).await;
    pilot
        .fly_direct(rates(0.0, 0.0), Duration::from_millis(500), |_| false)
        .await;
    let clockwise = measured_rate(pilot, rates(0.0, 1.5), 1.0, 2).await;
    pilot
        .fly_direct(rates(0.0, 0.0), Duration::from_millis(500), |_| false)
        .await;
    tracing::info!(
        roll_right = right,
        roll_left = left,
        yaw_clockwise = clockwise,
        "body rates for 1, -1, and 1.5 rad/s demands"
    );
    assert!(
        (right - 1.0).abs() <= 0.3,
        "roll rate {right:.2} for 1 rad/s"
    );
    assert!(
        (left + 1.0).abs() <= 0.3,
        "roll rate {left:.2} for -1 rad/s"
    );
    assert!(
        (clockwise - 1.5).abs() <= 0.4,
        "yaw rate {clockwise:.2} for 1.5 rad/s"
    );
}

/// Direct flight in ANGLE: an attitude demand produces that attitude.
async fn direct_attitude(pilot: &mut Pilot) {
    let heading = pilot
        .fly_direct(rates(0.0, 0.0), Duration::ZERO, |_| true)
        .await
        .heading as f32;
    let roll = 10f32.to_radians();
    // Z-Y-X: heading about down, then the roll about forward.
    let (sy, cy) = (heading / 2.0).sin_cos();
    let (sr, cr) = (roll / 2.0).sin_cos();
    let attitude = ControlIntent::AttitudeThrust(AttitudeThrustIntent {
        frame: ReferenceFrame::LocalNed,
        qw: cy * cr,
        qx: cy * sr,
        qy: sy * sr,
        qz: sy * cr,
        thrust: HOVER_THRUST,
    });
    let held = pilot
        .fly_direct(attitude, Duration::from_millis(1_500), |_| false)
        .await;
    tracing::info!(
        roll_deg = held.roll_deg,
        "attitude for a 10 deg roll demand"
    );
    assert!(
        (held.roll_deg - 10.0).abs() <= 3.0,
        "rolled {:.1} deg",
        held.roll_deg
    );
}

/// Back on the assisted scope the quad holds where it is, level.
async fn back_to_assisted(pilot: &mut Pilot) {
    let entry = pilot.fly([0.0; 4], Duration::ZERO, |_| true).await;
    let settled = pilot.fly([0.0; 4], Duration::from_secs(4), |_| false).await;
    tracing::info!(
        entry_up = entry.up,
        settled_up = settled.up,
        tilt_deg = settled.tilt_deg,
        "assisted hold after direct flight"
    );
    assert!(settled.up > 1.0, "sank to {:.2} m", settled.up);
    assert!(
        settled.tilt_deg < 5.0,
        "still tilted {:.1} deg",
        settled.tilt_deg
    );
}

async fn forward(pilot: &mut Pilot) {
    let start = pilot.fly([0.0; 4], Duration::ZERO, |_| true).await;
    let (sin, cos) = start.heading.sin_cos();
    let along = |r: &Reading| (r.north - start.north) * cos + (r.east - start.east) * sin;
    let moved = pilot
        .fly([2.0, 0.0, 0.0, 0.0], Duration::from_secs(8), |r| {
            along(r) >= 8.0
        })
        .await;
    let forward = along(&moved);
    tracing::info!(forward_m = forward, "flew forward on a forward intent");
    assert!(forward >= 5.0, "moved forward only {forward:.2} m");
}

async fn yaw(pilot: &mut Pilot) {
    pilot.fly([0.0; 4], Duration::from_secs(4), |_| false).await;
    let before = pilot.fly([0.0; 4], Duration::ZERO, |_| true).await;
    let yawed = pilot
        .fly([0.0, 0.0, 0.0, 1.0], Duration::from_secs(3), |r| {
            heading_change(before.heading, r.heading) >= 60.0
        })
        .await;
    let turned = heading_change(before.heading, yawed.heading);
    tracing::info!(turned_deg = turned, "yawed on a yaw intent");
    assert!(turned >= 45.0, "yawed only {turned:.1} deg");
}

async fn land_and_disarm(pilot: &mut Pilot) {
    pilot.fly([0.0; 4], Duration::from_secs(2), |_| false).await;
    let landed = pilot
        .fly([0.0, 0.0, 1.0, 0.0], Duration::from_secs(20), |r| {
            r.up < 0.15
        })
        .await;
    pilot
        .fly([0.0, 0.0, 1.0, 0.0], Duration::from_secs(2), |_| false)
        .await;
    tracing::info!(up = landed.up, "descended");
    let verdict = pilot.action(ControlAction::Disarm).await;
    tracing::info!(%verdict, "disarm");
    let disarmed = pilot
        .fly([0.0; 4], Duration::from_secs(3), |r| !r.armed)
        .await;
    assert!(!disarmed.armed, "Betaflight did not disarm");
}
