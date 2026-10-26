//! The recovery contract end to end over the simulated multicopter: a real
//! lane flies a survey through the engine actor, and an operator connection
//! takes the motion scope by transfer. While the operator holds it, nothing
//! moves the vehicle against them. A hand-back resumes the work. A closed
//! connection without a hand-back starts the approved way home, which an
//! expired approval's own way home also waits for.

use std::time::Duration;

use pilotage_adapter_sim_multicopter::{
    LaunchPoint, SimConfig, SimMulticopterAdapter, TileSourceConfig,
};
use pilotage_mission::{ExecutionTarget, Lane, LaneConfig, OwnshipSource, SurveyDefaults};
use pilotage_protocol::wire;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use wire::MissionLaneState as State;
use wire::mission_intent::Action;

mod operator;

use super::super::journal::LaneJournal;
use super::super::{LaneRuntimeConfig, LaneTask, PROGRESS_INTERVAL, unix_now_ms};
use super::hand_back::{approval, fetched};
use super::restart::send;
use crate::runtime::automation::MissionPlan;
use crate::runtime::engine_actor::{ENGINE_QUEUE_CAPACITY, EngineActor, ToEngine};
use crate::runtime::operations::{self, OperationHost};
use crate::runtime::{AutomationStatus, HOST_VEHICLE};
use operator::Operator;

const HOUR_MS: u64 = 3_600_000;
const HEIGHT_M: f64 = 15.0;
/// Virtual time a phase of a test may take; the clock is paused, so a
/// stalled rig fails at this deadline without costing wall time.
const PHASE: Duration = Duration::from_secs(900);
const LAUNCH: LaunchPoint = LaunchPoint {
    lat_deg: 40.3317,
    lon_deg: -74.6745,
    alt_m: 40.0,
};
const OPERATOR: u64 = 77;

fn point(latitude_deg: f64, longitude_deg: f64) -> wire::GeoPoint {
    wire::GeoPoint {
        latitude_deg,
        longitude_deg,
    }
}

/// A two-line survey 60 m north and 64 m east of the launch point.
fn survey() -> wire::MissionObjective {
    let north = 60.0 / 111_320.0;
    let east = 64.0 / (111_320.0 * LAUNCH.lat_deg.to_radians().cos());
    let (lat, lon) = (
        LAUNCH.lat_deg,
        LAUNCH.lon_deg + 20.0 / (111_320.0 * LAUNCH.lat_deg.to_radians().cos()),
    );
    wire::MissionObjective {
        objective_id: "survey-sim".to_owned(),
        revision: 1,
        title: "recovery survey".to_owned(),
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area: vec![
                    point(lat, lon),
                    point(lat + north, lon),
                    point(lat + north, lon + east),
                    point(lat, lon + east),
                ],
                launch: Some(point(LAUNCH.lat_deg, LAUNCH.lon_deg)),
                height_m: HEIGHT_M,
                cruise_mps: 8.0,
                return_to_launch: true,
                ..wire::SurveyObjective::default()
            },
        )),
        ..wire::MissionObjective::default()
    }
}

/// The engine actor over the simulated multicopter, and a lane that flies
/// on it as the host's lane does.
struct Rig {
    engine_tx: mpsc::Sender<ToEngine>,
    actor: JoinHandle<()>,
    task: LaneTask,
}

fn rig() -> Rig {
    let start = Instant::now();
    let mut vehicle = SimConfig::new(LAUNCH);
    vehicle.name = "recovery-test".to_owned();
    vehicle.camera.tiles = TileSourceConfig::Procedural;
    let adapter = SimMulticopterAdapter::new(HOST_VEHICLE, vehicle);
    // The test operator streams no control frames, so its link would read as
    // lost after the default silence; a closed connection is what ends it here.
    let engine = pilotage_session::SessionEngine::new(
        pilotage_adapter_api::VehicleAdapter::capabilities(&adapter),
        pilotage_timing::StalenessPolicy::new(crate::runtime::MAX_CONTROL_AGE),
        pilotage_session::SessionConfig::new(
            pilotage_protocol::SESSION_PROTOCOL_VERSION,
            env!("CARGO_PKG_VERSION"),
        )
        .with_holder_silence(Duration::from_secs(24 * 3600)),
    );
    let (engine_tx, engine_rx) = mpsc::channel(ENGINE_QUEUE_CAPACITY);
    let actor = tokio::spawn(EngineActor::new(engine, adapter, start).run(engine_rx));
    let operations = OperationHost {
        sources: None,
        reports: None,
    };
    let config = LaneRuntimeConfig {
        lane: LaneConfig {
            name: "SIM-1".to_owned(),
            simulated: true,
            execution_strategy: "host-guided-velocity".to_owned(),
            template: MissionPlan::lane_template(
                OwnshipSource::vehicle_estimate(),
                ExecutionTarget::Simulator,
            ),
            defaults: SurveyDefaults::default(),
            usable_energy_wh: None,
            cruise_power_w: None,
            recovery_policy: "hold in place".to_owned(),
            judged: false,
            evidence_window_ms: 1_000,
            executors: operations::registry(&operations),
        },
        decision: None,
        assess_every: Duration::from_secs(6),
        judge_height_m: 0.0,
        planar_pose_is_truth: false,
        operations,
        journal: None,
    };
    let lane = Lane::new(config.lane.clone());
    let task = LaneTask::new(
        config,
        engine_tx.clone(),
        start,
        lane,
        LaneJournal::new(None, None),
    );
    Rig {
        engine_tx,
        actor,
        task,
    }
}

impl Rig {
    /// One tick of the lane, as the host's loop runs it.
    async fn tick(&mut self) {
        let host_ms = self.task.host_ms();
        self.task.lane.on_host_clock(host_ms);
        if let Some(effect) = self.task.lane.on_clock(unix_now_ms()) {
            self.task.apply(effect).await;
        }
        self.task.on_progress().await;
        self.task.record().await;
        tokio::time::sleep(PROGRESS_INTERVAL).await;
    }

    fn run_status(&self) -> AutomationStatus {
        self.task
            .run
            .as_ref()
            .map(|run| run.status.borrow().clone())
            .unwrap_or_default()
    }

    /// Ticks until `done` holds for the lane task.
    async fn until(&mut self, what: &str, done: impl Fn(&LaneTask) -> bool) {
        let deadline = Instant::now() + PHASE;
        while !done(&self.task) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: lane {:?} {}; run {:?}",
                self.task.lane.state(),
                self.task.lane.status().detail,
                self.run_status()
            );
            self.tick().await;
        }
    }

    /// Proposes, approves under `on_control_lost` and starts the survey,
    /// then flies until it is on its way.
    async fn flying(&mut self, on_control_lost: wire::MissionControlLossPolicy) {
        send(&mut self.task, Action::Propose(survey())).await;
        let proposed = self.task.lane.status().proposal_digest;
        let digest = fetched(&mut self.task, proposed);
        let now = unix_now_ms();
        send(
            &mut self.task,
            Action::Approve(approval(&digest, now + HOUR_MS, on_control_lost)),
        )
        .await;
        send(
            &mut self.task,
            Action::Start(wire::MissionStart {
                proposal_digest: digest,
            }),
        )
        .await;
        assert_eq!(
            self.task.lane.state(),
            State::Running,
            "{}",
            self.task.lane.status().detail
        );
        self.until("the survey enroute", |task| {
            task.run.as_ref().is_some_and(|run| {
                run.status.borrow().mission_state == Some(pilotage_mission::MissionState::Enroute)
            })
        })
        .await;
    }

    async fn shutdown(mut self) {
        if let Some(run) = self.task.run.take() {
            run.controls
                .send(super::super::RunControl::Abort)
                .await
                .ok();
            run.handle.await.ok();
        }
        drop(self.task);
        drop(self.engine_tx);
        // An operator still connected keeps the actor's channel open: stop the
        // actor rather than wait for the channel to close.
        self.actor.abort();
        self.actor.await.ok();
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .expect("a runtime")
}

fn awaiting_hand_back(task: &LaneTask) -> bool {
    task.run
        .as_ref()
        .is_some_and(|run| run.status.borrow().awaiting_hand_back)
}

#[test]
fn a_closed_operator_connection_starts_the_approved_way_home() {
    runtime().block_on(async {
        let mut rig = rig();
        rig.flying(wire::MissionControlLossPolicy::ReturnHome).await;
        let mut operator = Operator::connect(&rig, OPERATOR).await;
        operator.take_control().await;
        rig.until("the mission to lose control", awaiting_hand_back)
            .await;
        let runs = rig.task.runs;
        for _ in 0..20 {
            rig.tick().await;
        }
        assert_eq!(rig.task.runs, runs, "nothing moves against the operator");
        assert_eq!(rig.task.lane.state(), State::Running);
        operator.disconnect().await;
        rig.until("the way home to start", |task| task.runs != runs)
            .await;
        assert!(
            rig.task.lane.status().detail.contains("launch point"),
            "{}",
            rig.task.lane.status().detail
        );
        rig.until("the way home to end", |task| {
            task.lane.state() != State::Running
        })
        .await;
        assert_eq!(
            rig.task.lane.state(),
            State::Complete,
            "{}",
            rig.task.lane.status().detail
        );
        rig.shutdown().await;
    });
}

#[test]
fn without_an_approved_recovery_a_closed_connection_leaves_the_mission_held() {
    runtime().block_on(async {
        let mut rig = rig();
        rig.flying(wire::MissionControlLossPolicy::Hold).await;
        let mut operator = Operator::connect(&rig, OPERATOR).await;
        operator.take_control().await;
        rig.until("the mission to lose control", awaiting_hand_back)
            .await;
        let runs = rig.task.runs;
        operator.disconnect().await;
        rig.until("the run to see the freed lease", |task| {
            task.run
                .as_ref()
                .is_some_and(|run| run.status.borrow().control_freed)
        })
        .await;
        for _ in 0..40 {
            rig.tick().await;
        }
        assert_eq!(rig.task.runs, runs, "no run starts");
        assert!(
            awaiting_hand_back(&rig.task),
            "the work waits for a hand-back"
        );
        assert_eq!(
            rig.run_status().lease_generation,
            None,
            "the mission holds no lease"
        );
        rig.shutdown().await;
    });
}

#[test]
fn a_hand_back_resumes_the_work_and_no_recovery_runs() {
    runtime().block_on(async {
        let mut rig = rig();
        rig.flying(wire::MissionControlLossPolicy::ReturnHome).await;
        let mut operator = Operator::connect(&rig, OPERATOR).await;
        let mission = operator.take_control().await;
        rig.until("the mission to lose control", awaiting_hand_back)
            .await;
        let runs = rig.task.runs;
        operator.hand_back(mission).await;
        rig.until("the hand-back", |task| !awaiting_hand_back(task))
            .await;
        let resumed = rig.run_status();
        assert!(resumed.lease_generation.is_some(), "{resumed:?}");
        for _ in 0..40 {
            rig.tick().await;
        }
        let flown = rig.run_status();
        assert_eq!(rig.task.runs, runs, "the same run carries on the work");
        assert_eq!(rig.task.lane.state(), State::Running);
        assert!(
            flown.frames_sent > resumed.frames_sent,
            "the work flies again"
        );
        assert_eq!(flown.frames_rejected, 0, "{flown:?}");
        rig.shutdown().await;
    });
}

#[test]
fn an_expiry_way_home_waits_for_the_operator_and_starts_when_they_leave() {
    runtime().block_on(async {
        let mut rig = rig();
        rig.flying(wire::MissionControlLossPolicy::Hold).await;
        let mut operator = Operator::connect(&rig, OPERATOR).await;
        operator.take_control().await;
        rig.until("the mission to lose control", awaiting_hand_back)
            .await;
        let runs = rig.task.runs;
        let effect = rig
            .task
            .lane
            .on_clock(unix_now_ms() + 2 * HOUR_MS)
            .expect("the expiry names the way home");
        rig.task.apply(effect).await;
        for _ in 0..20 {
            rig.tick().await;
        }
        assert_eq!(
            rig.task.runs, runs,
            "the way home waits while the operator holds control"
        );
        operator.disconnect().await;
        rig.until("the way home to start", |task| task.runs != runs)
            .await;
        rig.until("the way home to end", |task| {
            task.lane.state() != State::Running
        })
        .await;
        assert_eq!(
            rig.task.lane.state(),
            State::Complete,
            "{}",
            rig.task.lane.status().detail
        );
        rig.shutdown().await;
    });
}
