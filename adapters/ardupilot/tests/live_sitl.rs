#![allow(clippy::expect_used, clippy::panic)]

//! Real flights against a running ArduCopter SITL, through the public
//! adapter API only. The velocity flight arms, climbs on a climb intent,
//! flies forward, yaws, and lands on a disarm. The direct flight switches
//! a hover into body-rate control and back, and measures the rates the FC
//! flies. Ignored by default; they need a SITL whose serial 0
//! sends to the endpoint in `PILOTAGE_ARDUPILOT_SITL`, for example
//!
//! ```text
//! arducopter --model + -I3 --serial0 udpclient:127.0.0.1:14593 ...
//! PILOTAGE_ARDUPILOT_SITL=127.0.0.1:14593 \
//!   cargo test -p pilotage-adapter-ardupilot --test live_sitl -- --ignored --nocapture \
//!   --test-threads 1
//! ```

use std::time::Duration;

use pilotage_adapter_api::{TelemetrySample, VehicleAdapter};
use pilotage_adapter_ardupilot::{
    ArduPilotAdapter, ArduPilotConfig, ArduPilotProfile, DIRECT_SCOPE, FLIGHT_SCOPE,
};
use pilotage_protocol::{
    BodyRateIntent, ControlAction, ControlIntent, ControlPayload, Generation, ReferenceFrame,
    ScopeId, ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use tracing::info;

const ENDPOINT_ENV: &str = "PILOTAGE_ARDUPILOT_SITL";
/// The control tick: the rate a client streams intents at.
const TICK: Duration = Duration::from_millis(50);

/// What one telemetry sample says about the vehicle.
#[derive(Debug, Clone, Copy)]
struct Observed {
    north: f64,
    east: f64,
    up: f64,
    heading: f64,
    armed: bool,
    /// The receiver fix, latitude and longitude in degrees.
    fix: Option<(f64, f64)>,
    battery: Option<f32>,
    /// Body rates (roll, pitch, yaw), rad/s.
    rates: [f32; 3],
    /// Vertical speed, m/s, + = up.
    climb: f64,
}

fn observe(sample: Option<&TelemetrySample>) -> Option<Observed> {
    let sample = sample?;
    let pose = sample.pose?;
    let down = sample.avionics?.kinematics?.pos_ned_m[2];
    let pilotage_adapter_api::AvionicsAttitudeSample::Canonical {
        rates_rps: Some(rates),
        ..
    } = sample.avionics?.attitude?
    else {
        return None;
    };
    Some(Observed {
        north: pose.x,
        east: pose.y,
        up: -f64::from(down),
        heading: pose.heading,
        armed: sample.fc_state.is_some_and(|state| state.arm_state == 2),
        fix: sample
            .avionics?
            .geodetic
            .map(|fix| (fix.position.latitude_deg, fix.position.longitude_deg)),
        battery: sample.battery.and_then(|battery| battery.voltage_v),
        rates,
        climb: -f64::from(sample.avionics?.kinematics?.vel_ned_mps[2]),
    })
}

struct Pilot {
    adapter: ArduPilotAdapter,
    ticker: tokio::time::Interval,
    sequence: u32,
    latest: Option<Observed>,
}

impl Pilot {
    fn frame(
        &mut self,
        intent: Option<ControlIntent>,
        actions: Vec<ControlAction>,
    ) -> ScopedControlFrame {
        self.sequence = self.sequence.wrapping_add(1);
        let scope = match intent {
            None | Some(ControlIntent::Velocity(_)) => FLIGHT_SCOPE,
            Some(_) => DIRECT_SCOPE,
        };
        ScopedControlFrame {
            timing: None,
            admitted_timing: None,
            action_ids: (1..=actions.len() as u32).collect(),
            session: SessionId::new(1),
            vehicle: VehicleId::new(1),
            scope: ScopeId::new(scope),
            generation: Generation::new(1),
            sequence: SequenceNum::new(self.sequence),
            sampled_at: pilotage_timing::MonoTimestamp::from_nanos(0),
            profile_revision: 1,
            activation_revision: 0,
            payload: ControlPayload::default(),
            intent,
            actions,
        }
    }

    /// One control tick: optionally a frame, then a telemetry sample.
    async fn tick(
        &mut self,
        intent: Option<ControlIntent>,
        actions: Vec<ControlAction>,
    ) -> Option<Observed> {
        self.ticker.tick().await;
        if intent.is_some() || !actions.is_empty() {
            let frame = self.frame(intent, actions);
            self.adapter.apply_control(&frame);
        }
        let batch = self.adapter.sample_telemetry();
        if let Some(observed) = observe(batch.samples.first()) {
            self.latest = Some(observed);
        }
        self.latest
    }

    /// Arms, again every few seconds while ArduCopter's pre-arm checks
    /// are still settling, until the FC reports armed.
    async fn arm(&mut self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut attempt = 0_u32;
        loop {
            let actions = if attempt.is_multiple_of(60) {
                vec![ControlAction::Arm]
            } else {
                vec![]
            };
            attempt = attempt.wrapping_add(1);
            if self.tick(None, actions).await.is_some_and(|o| o.armed) {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "never armed");
        }
        info!(ticks = attempt, "armed");
    }

    /// Ticks with `intent` until `done` holds, failing after `limit`.
    async fn fly_until(
        &mut self,
        what: &str,
        limit: Duration,
        intent: Option<ControlIntent>,
        done: impl Fn(Observed) -> bool,
    ) -> Observed {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            if let Some(observed) = self.tick(intent, vec![]).await
                && done(observed)
            {
                info!(what, ?observed, "phase complete");
                return observed;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{what}: not reached within {limit:?}; last {:?}",
                self.latest
            );
        }
    }
}

fn velocity(vx: f32, vz: f32, yaw_rate: f32) -> Option<ControlIntent> {
    Some(ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy: 0.0,
        vz,
        yaw_rate,
    }))
}

fn body_rates(rates: [f32; 3], thrust: f32) -> Option<ControlIntent> {
    Some(ControlIntent::BodyRate(BodyRateIntent {
        roll_rate: rates[0],
        pitch_rate: rates[1],
        yaw_rate: rates[2],
        thrust,
    }))
}

fn wrapped_delta(from: f64, to: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    ((to - from + std::f64::consts::PI).rem_euclid(tau)) - std::f64::consts::PI
}

async fn start_pilot(endpoint: std::net::SocketAddr) -> Pilot {
    let mut config = ArduPilotConfig::new(ArduPilotProfile::Simulation);
    config.endpoint = endpoint;
    let adapter = ArduPilotAdapter::start(VehicleId::new(1), config)
        .await
        .expect("the adapter starts");
    Pilot {
        adapter,
        ticker: tokio::time::interval(TICK),
        sequence: 0,
        latest: None,
    }
}

async fn flight(endpoint: std::net::SocketAddr) {
    let mut pilot = start_pilot(endpoint).await;

    let ground = pilot
        .fly_until("estimate ready", Duration::from_secs(120), None, |o| {
            o.fix.is_some() && o.battery.is_some()
        })
        .await;

    pilot.arm().await;

    let climbed = pilot
        .fly_until(
            "climb",
            Duration::from_secs(40),
            velocity(0.0, -1.5, 0.0),
            |o| o.up - ground.up >= 2.0,
        )
        .await;
    let start = pilot
        .fly_until("settle", TICK, velocity(0.0, 0.0, 0.0), |_| true)
        .await;
    let forward = pilot
        .fly_until(
            "forward",
            Duration::from_secs(30),
            velocity(3.0, 0.0, 0.0),
            |o| (o.north - start.north).hypot(o.east - start.east) >= 5.0,
        )
        .await;
    let before_yaw = pilot
        .fly_until("brake", TICK, velocity(0.0, 0.0, 0.0), |_| true)
        .await;
    let yawed = pilot
        .fly_until(
            "yaw",
            Duration::from_secs(20),
            velocity(0.0, 0.0, 0.9),
            |o| wrapped_delta(before_yaw.heading, o.heading).abs() >= 45_f64.to_radians(),
        )
        .await;

    pilot.tick(None, vec![ControlAction::Disarm]).await;
    let landed = pilot
        .fly_until("land and disarm", Duration::from_secs(120), None, |o| {
            !o.armed
        })
        .await;

    info!(
        climb_m = climbed.up - ground.up,
        forward_m = (forward.north - start.north).hypot(forward.east - start.east),
        yaw_deg = wrapped_delta(before_yaw.heading, yawed.heading).to_degrees(),
        landed_up_m = landed.up - ground.up,
        fix = ?ground.fix,
        battery_v = ?ground.battery,
        "flight complete"
    );
}

/// Runs `flight` against the SITL in `PILOTAGE_ARDUPILOT_SITL`.
fn against_sitl<F: std::future::Future<Output = ()>>(
    flight: impl FnOnce(std::net::SocketAddr) -> F,
) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init()
        .ok();
    let endpoint = std::env::var(ENDPOINT_ENV)
        .unwrap_or_else(|_| panic!("set {ENDPOINT_ENV} to the SITL's udpclient address"))
        .parse()
        .expect("a socket address");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(flight(endpoint));
}

#[test]
#[ignore = "flies a running ArduCopter SITL; set PILOTAGE_ARDUPILOT_SITL"]
fn arm_climb_forward_yaw_and_land_against_sitl() {
    against_sitl(flight);
}

#[test]
#[ignore = "flies a running ArduCopter SITL; set PILOTAGE_ARDUPILOT_SITL"]
fn direct_body_rates_and_family_switches_against_sitl() {
    against_sitl(direct::flight);
}

#[path = "live_sitl/direct.rs"]
mod direct;
