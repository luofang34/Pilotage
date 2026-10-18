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

use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    CommandOutcome, Disposition, LinkLossPolicy, TelemetryBatch, VehicleAdapter,
};
use pilotage_adapter_px4::{FLIGHT_SCOPE, Px4Adapter, Px4Airframe, Px4Config, Px4Profile};
use pilotage_protocol::{
    ControlAction, ControlIntent, ControlPayload, Generation, IntentFamily, ReferenceFrame,
    ScopeId, ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;
use tracing::info;

const VEHICLE: VehicleId = VehicleId::new(1);
/// The operator control cadence.
const FRAME_PERIOD: Duration = Duration::from_millis(50);

/// The flight state the test reads off each telemetry batch.
#[derive(Debug, Clone, Copy)]
struct Observed {
    down_m: f32,
    heading_rad: f64,
}

fn observe(batch: &TelemetryBatch) -> Option<Observed> {
    batch.samples.iter().find_map(|sample| {
        let kinematics = sample.avionics.as_ref()?.kinematics.as_ref()?;
        Some(Observed {
            down_m: kinematics.pos_ned_m[2],
            heading_rad: sample.pose?.heading,
        })
    })
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn live_config() -> Px4Config {
    let mut config = Px4Config::new(Px4Profile::Simulation);
    config.airframe = Px4Airframe::FixedWing;
    config.telemetry_endpoint = env_or("PILOTAGE_PX4_ADDR", "127.0.0.1:14550")
        .parse()
        .expect("PILOTAGE_PX4_ADDR");
    config.stream_command_endpoint = env_or("PILOTAGE_PX4_GCS_ADDR", "127.0.0.1:18570")
        .parse()
        .expect("PILOTAGE_PX4_GCS_ADDR");
    config.command_endpoint = env_or("PILOTAGE_PX4_FC_ADDR", "127.0.0.1:14580")
        .parse()
        .expect("PILOTAGE_PX4_FC_ADDR");
    config.system_id = env_or("PILOTAGE_PX4_SYSTEM_ID", "1")
        .parse()
        .expect("PILOTAGE_PX4_SYSTEM_ID");
    config
}

/// One operator frame: a typed velocity intent and optional actions.
struct Operator {
    sequence: u32,
}

impl Operator {
    fn frame(
        &mut self,
        velocity: VelocityIntent,
        actions: Vec<ControlAction>,
    ) -> ScopedControlFrame {
        self.sequence = self.sequence.wrapping_add(1);
        ScopedControlFrame {
            session: SessionId::new(1),
            vehicle: VEHICLE,
            scope: ScopeId::new(FLIGHT_SCOPE),
            generation: Generation::new(1),
            sequence: SequenceNum::new(self.sequence),
            sampled_at: MonoTimestamp::from_nanos(0),
            profile_revision: 1,
            activation_revision: 0,
            payload: ControlPayload::default(),
            intent: Some(ControlIntent::Velocity(velocity)),
            actions,
            action_ids: vec![],
        }
    }
}

fn velocity(vx: f32, vz: f32, yaw_rate: f32) -> VelocityIntent {
    VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy: 0.0,
        vz,
        yaw_rate,
    }
}

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

/// Runs the operator loop until `done` holds for the latest observation or
/// `budget` lapses; returns the last observation and the last disposition.
async fn fly_until(
    adapter: &mut Px4Adapter,
    operator: &mut Operator,
    demand: VelocityIntent,
    budget: Duration,
    mut done: impl FnMut(Observed, &Disposition) -> bool,
) -> (Option<Observed>, Disposition) {
    let mut ticker = tokio::time::interval(FRAME_PERIOD);
    let deadline = Instant::now() + budget;
    let mut last = None;
    let mut disposition = Disposition::Accepted;
    while Instant::now() < deadline {
        ticker.tick().await;
        disposition = adapter
            .apply_control(&operator.frame(demand, vec![]))
            .disposition;
        if let Some(observed) = observe(&adapter.sample_telemetry()) {
            last = Some(observed);
            if done(observed, &disposition) {
                break;
            }
        }
    }
    (last, disposition)
}

/// Presses ARM, as an operator would, until PX4 reports itself armed with
/// an accepted arm command. The adapter refuses the press until PX4 echoes
/// every pinned parameter, and PX4 refuses the arm while its own preflight
/// checks still fail.
async fn arm(adapter: &mut Px4Adapter, operator: &mut Operator) -> Duration {
    let started = Instant::now();
    let mut ticker = tokio::time::interval(FRAME_PERIOD);
    let mut last_press: Option<Instant> = None;
    let mut press_accepted = false;
    while started.elapsed() < Duration::from_secs(90) {
        ticker.tick().await;
        let batch = adapter.sample_telemetry();
        let fc_state = batch.samples.iter().find_map(|sample| sample.fc_state);
        let armed = fc_state.is_some_and(|state| state.arm_state == 2);
        let answer = fc_state
            .and_then(|state| state.last_command)
            .filter(|ack| ack.arm)
            .map(|ack| ack.outcome);
        let refused = answer.is_some_and(CommandOutcome::is_refusal);
        if press_accepted && armed && answer == Some(CommandOutcome::Accepted) {
            return started.elapsed();
        }
        let due = last_press.is_none_or(|at| at.elapsed() >= Duration::from_secs(2));
        let actions = if due && (!press_accepted || refused) {
            last_press = Some(Instant::now());
            vec![ControlAction::Arm]
        } else {
            vec![]
        };
        let outcome = adapter.apply_control(&operator.frame(velocity(0.0, 0.0, 0.0), actions));
        if let Some(result) = outcome.action_results.first() {
            press_accepted = result.accepted;
            info!(accepted = result.accepted, detail = %result.detail, "ARM pressed");
        }
    }
    panic!("PX4 never armed");
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
    let mut adapter = Px4Adapter::start(VEHICLE, live_config())
        .await
        .expect("adapter");
    let mut operator = Operator { sequence: 0 };
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
    let mut ticker = tokio::time::interval(FRAME_PERIOD);
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
    tracing_subscriber::fmt().with_test_writer().init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(flight());
}
