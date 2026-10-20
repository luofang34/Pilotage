//! The operator loop the PX4 SITL flight tests share: endpoints from the
//! environment, typed frames, ARM until PX4 answers, and a 20 Hz control
//! loop that observes the estimate after every frame.

// Each flight test is its own crate and uses a different subset of this
// module.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use pilotage_adapter_api::{CommandOutcome, Disposition, TelemetryBatch, VehicleAdapter};
use pilotage_adapter_px4::{FLIGHT_SCOPE, Px4Adapter, Px4Airframe, Px4Config, Px4Profile};
use pilotage_protocol::{
    ControlAction, ControlIntent, ControlPayload, Generation, ReferenceFrame, ScopeId,
    ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;
use tracing::info;

pub(crate) const VEHICLE: VehicleId = VehicleId::new(1);
/// The operator control cadence.
pub(crate) const FRAME_PERIOD: Duration = Duration::from_millis(50);

/// The flight state the tests read off each telemetry batch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Observed {
    pub(crate) down_m: f32,
    pub(crate) heading_rad: f64,
    /// Attitude quaternion (w, x, y, z), body FRD from local NED.
    pub(crate) quat_wxyz: [f32; 4],
    /// Body rates (roll, pitch, yaw), rad/s.
    pub(crate) rates_rps: [f32; 3],
}

pub(crate) fn observe(batch: &TelemetryBatch) -> Option<Observed> {
    batch.samples.iter().find_map(|sample| {
        let avionics = sample.avionics.as_ref()?;
        let kinematics = avionics.kinematics.as_ref()?;
        let pilotage_adapter_api::AvionicsAttitudeSample::Canonical {
            quat_wxyz,
            rates_rps: Some(rates_rps),
            ..
        } = avionics.attitude?
        else {
            return None;
        };
        Some(Observed {
            down_m: kinematics.pos_ned_m[2],
            heading_rad: sample.pose?.heading,
            quat_wxyz,
            rates_rps,
        })
    })
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

/// The adapter configuration for the PX4 instance the environment names.
pub(crate) fn live_config(airframe: Px4Airframe) -> Px4Config {
    let mut config = Px4Config::new(Px4Profile::Simulation);
    config.airframe = airframe;
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

/// One typed demand and the scope it rides on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Demand {
    pub(crate) scope: &'static str,
    pub(crate) intent: ControlIntent,
}

/// A body-FRD velocity demand on the flight scope.
pub(crate) fn velocity(vx: f32, vz: f32, yaw_rate: f32) -> Demand {
    Demand {
        scope: FLIGHT_SCOPE,
        intent: ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx,
            vy: 0.0,
            vz,
            yaw_rate,
        }),
    }
}

/// The operator's frame sequence.
#[derive(Debug, Default)]
pub(crate) struct Operator {
    sequence: u32,
}

impl Operator {
    pub(crate) fn frame(
        &mut self,
        demand: Demand,
        actions: Vec<ControlAction>,
    ) -> ScopedControlFrame {
        self.sequence = self.sequence.wrapping_add(1);
        ScopedControlFrame {
            timing: None,
            admitted_timing: None,
            session: SessionId::new(1),
            vehicle: VEHICLE,
            scope: ScopeId::new(demand.scope),
            generation: Generation::new(1),
            sequence: SequenceNum::new(self.sequence),
            sampled_at: MonoTimestamp::from_nanos(0),
            profile_revision: 1,
            activation_revision: 0,
            payload: ControlPayload::default(),
            intent: Some(demand.intent),
            actions,
            action_ids: vec![],
        }
    }
}

/// Runs the operator loop until `done` holds for the latest observation or
/// `budget` lapses; returns the last observation and the last disposition.
pub(crate) async fn fly_until(
    adapter: &mut Px4Adapter,
    operator: &mut Operator,
    demand: Demand,
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
/// an accepted arm command. The adapter can refuse the press (a fixed wing
/// waits for its pinned parameters), and PX4 refuses the arm while its own
/// preflight checks still fail.
pub(crate) async fn arm(adapter: &mut Px4Adapter, operator: &mut Operator) -> Duration {
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

/// Starts tracing and runs `flight` on an explicit multi-thread runtime.
pub(crate) fn run(flight: impl std::future::Future<Output = ()>) {
    tracing_subscriber::fmt().with_test_writer().init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(flight);
}
