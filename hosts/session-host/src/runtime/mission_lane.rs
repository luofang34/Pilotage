//! The host's mission lane (ADR-0052): one per vehicle, at the vehicle's
//! control endpoint. It answers client intents through the sans-IO
//! [`pilotage_mission::Lane`], runs each approved plan as its own mission
//! principal, counts the pictures each survey line acquires, asks the
//! judgment service about a sample of them, and publishes its status.

mod config;
mod frame_tap;
mod judge;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pilotage_adapter_api::RawVideoFrame;
use pilotage_mission::{
    FrameAssessment, IntentContext, Lane, LaneEffect, MissionState, Requester, RunProgress,
    Steering,
};
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{info, warn};

use super::automation::{self, AutomationStatus, MissionPlan, RunControl};
use super::engine_actor::ToEngine;

pub(crate) use config::LaneRuntimeConfig;
pub(crate) use frame_tap::tap;

/// Intents and pictures the lane can fall behind by.
const INBOUND_QUEUE: usize = 32;
/// How often the lane reads its run's progress.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
/// The status is published at least this often, for late joiners.
const STATUS_REFRESH: Duration = Duration::from_secs(5);
/// Run principals take keys below the mission and observer principals.
const RUN_CLIENT_BASE: u64 = u64::MAX - 16;

/// What reaches the lane.
#[derive(Debug)]
pub(crate) enum LaneInbound {
    /// An intent from a client.
    Intent {
        /// The sender.
        client: ClientKey,
        /// Who the sender is.
        requester: pilotage_mission::Requester,
        /// The intent.
        intent: Box<wire::MissionIntent>,
    },
    /// A sampled picture from the vehicle's camera.
    Frame(Arc<RawVideoFrame>),
}

/// The run the lane started.
struct ActiveRun {
    handle: JoinHandle<()>,
    status: watch::Receiver<AutomationStatus>,
    controls: mpsc::Sender<RunControl>,
    /// The run was told to stop and release the vehicle.
    stopping: bool,
}

/// Spawns the lane and registers it with the engine actor. Returns the
/// sender the frame tap feeds and the lane's task.
pub(crate) async fn spawn(
    engine_tx: &mpsc::Sender<ToEngine>,
    start: Instant,
    config: LaneRuntimeConfig,
) -> (mpsc::Sender<LaneInbound>, JoinHandle<()>) {
    let (sender, inbound) = mpsc::channel(INBOUND_QUEUE);
    let registered = engine_tx
        .send(ToEngine::MissionLane {
            sender: sender.clone(),
        })
        .await;
    if registered.is_err() {
        warn!("the engine actor is gone; the mission lane does not start");
    }
    info!(lane = %config.lane.name, simulated = config.lane.simulated, judge = config.decision.is_some(), "mission lane ready");
    let engine = engine_tx.clone();
    let handle = tokio::spawn(async move {
        LaneTask::new(config, engine, start).run(inbound).await;
        tracing::debug!(task = "mission-lane", "task exited");
    });
    (sender, handle)
}

struct LaneTask {
    lane: Lane,
    engine: mpsc::Sender<ToEngine>,
    start: Instant,
    planar_pose_is_truth: bool,
    run: Option<ActiveRun>,
    runs: u64,
    ids: u64,
    judge: Option<judge::Judge>,
    last_publish: Option<Instant>,
    steering: Steering,
}

impl LaneTask {
    fn new(config: LaneRuntimeConfig, engine: mpsc::Sender<ToEngine>, start: Instant) -> Self {
        Self {
            judge: config.decision.map(|provider| {
                judge::Judge::new(provider, config.assess_every, config.judge_height_m)
            }),
            lane: Lane::new(config.lane),
            engine,
            start,
            planar_pose_is_truth: config.planar_pose_is_truth,
            run: None,
            runs: 0,
            ids: 0,
            last_publish: None,
            steering: Steering::default(),
        }
    }

    async fn run(mut self, mut inbound: mpsc::Receiver<LaneInbound>) {
        let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut judged = self.judge.as_mut().map(judge::Judge::take_results);
        loop {
            tokio::select! {
                message = inbound.recv() => match message {
                    Some(LaneInbound::Intent { client, requester, mut intent }) => {
                        if !self.on_intent(client, &requester, &mut intent).await {
                            break;
                        }
                    }
                    Some(LaneInbound::Frame(frame)) => self.on_frame(frame),
                    None => break,
                },
                result = judge::next(&mut judged) => {
                    if let Some((task, judgment)) = result {
                        self.lane.on_assessment(&task, FrameAssessment {
                            usable: judgment.usable >= 0.5,
                            hazard: judgment.hazard >= 0.5,
                        });
                    }
                },
                _ = ticker.tick() => {
                    if let Some(effect) = self.lane.on_clock(unix_now_ms()) {
                        self.apply(effect).await;
                    }
                    self.on_progress().await;
                    if !self.publish(false).await {
                        break;
                    }
                }
            }
        }
        if let Some(run) = self.run.take() {
            run.controls.send(RunControl::Abort).await.ok();
            run.handle.await.ok();
        }
    }

    async fn on_intent(
        &mut self,
        client: ClientKey,
        requester: &Requester,
        intent: &mut wire::MissionIntent,
    ) -> bool {
        if let Err(reason) = self.steering.admit(intent, requester) {
            info!(request = %intent.request_id, %reason, "mission intent refused");
            let receipt = Steering::refusal(intent, &self.lane.status().lane, reason);
            return self
                .send(
                    Some(client),
                    wire::envelope::Payload::MissionReceipt(receipt),
                )
                .await;
        }
        self.ids = self.ids.wrapping_add(1);
        let now_unix_ms = unix_now_ms();
        let context = IntentContext {
            now_unix_ms,
            fresh_id: format!("{}-{now_unix_ms}-{}", self.lane.status().lane, self.ids),
        };
        let reply = self.lane.on_intent(intent, &context);
        if reply.receipt.outcome == wire::MissionReceiptOutcome::Accepted as i32 {
            self.steering.accepted(intent, requester);
        }
        info!(request = %reply.receipt.request_id, outcome = reply.receipt.outcome, reason = %reply.receipt.reason, "mission intent answered");
        let receipt = wire::envelope::Payload::MissionReceipt(reply.receipt);
        if !self.send(Some(client), receipt).await {
            return false;
        }
        if let Some(effect) = reply.effect {
            self.apply(effect).await;
        }
        self.publish(true).await
    }

    async fn apply(&mut self, effect: LaneEffect) {
        let control = match effect {
            LaneEffect::Start(order) => {
                if let Some(previous) = self.run.take() {
                    // The previous run released the vehicle or is releasing
                    // it; waiting for it keeps two principals from asking
                    // for the same lease.
                    previous.controls.send(RunControl::Abort).await.ok();
                    previous.handle.await.ok();
                }
                self.runs = self.runs.wrapping_add(1);
                let client = ClientKey::new(RUN_CLIENT_BASE.wrapping_sub(self.runs));
                let (controls, receiver) = mpsc::channel(4);
                let plan = MissionPlan::of_lane(order.plan, order.config);
                let (handle, status) = automation::spawn_mission_run(
                    &self.engine,
                    self.start,
                    plan,
                    self.planar_pose_is_truth,
                    client,
                    Some(receiver),
                );
                info!(run = %order.run_id, "mission run started");
                self.run = Some(ActiveRun {
                    handle,
                    status,
                    controls,
                    stopping: false,
                });
                return;
            }
            LaneEffect::Hold => RunControl::Hold,
            LaneEffect::Resume => RunControl::Resume,
            LaneEffect::Abort => RunControl::Abort,
        };
        if let Some(run) = self.run.as_ref() {
            run.controls.send(control).await.ok();
        }
    }

    async fn on_progress(&mut self) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let status = run.status.borrow().clone();
        let finished = run.handle.is_finished();
        if let Some(effect) = self.lane.on_progress(&run_progress(&status, finished)) {
            // A revision's checkpoint: its run replaces this one.
            self.apply(effect).await;
            return;
        }
        let Some(run) = self.run.as_mut() else {
            return;
        };
        // A run whose lane ended stops the vehicle in place and releases
        // it, so the next run of this lane can take the motion lease.
        let ended = !matches!(
            self.lane.state(),
            wire::MissionLaneState::Running
                | wire::MissionLaneState::HoldRequested
                | wire::MissionLaneState::Held
                | wire::MissionLaneState::Uncertain
        );
        if ended && !run.stopping && !finished {
            run.stopping = run.controls.try_send(RunControl::Abort).is_ok();
        }
        if finished {
            self.run = None;
        }
    }

    fn on_frame(&mut self, frame: Arc<RawVideoFrame>) {
        if let Some(task) = self.lane.on_frame()
            && let Some(judge) = self.judge.as_mut()
        {
            judge.offer(task, frame);
        }
    }

    /// Publishes the status when it changed, or when `force`, or when the
    /// refresh period passed. Returns `false` when the engine is gone.
    async fn publish(&mut self, force: bool) -> bool {
        let stale = self
            .last_publish
            .is_none_or(|at| at.elapsed() >= STATUS_REFRESH);
        let status = match self.lane.take_changed() {
            Some(status) => status,
            None if force || stale => self.lane.status(),
            None => return true,
        };
        self.last_publish = Some(Instant::now());
        self.send(None, wire::envelope::Payload::MissionStatus(status))
            .await
    }

    async fn send(&self, to: Option<ClientKey>, payload: wire::envelope::Payload) -> bool {
        self.engine
            .send(ToEngine::MissionPublish {
                to,
                payload: Box::new(payload),
            })
            .await
            .is_ok()
    }
}

/// Civil time now, milliseconds since the Unix epoch.
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The lane's view of its run: a run that exited, was fenced or closed,
/// or whose mission failed has ended.
fn run_progress(status: &AutomationStatus, finished: bool) -> RunProgress {
    RunProgress {
        target_waypoint: status.target_waypoint,
        plan_complete: status.plan_complete,
        ended: finished
            || status.fenced
            || status.closed
            || status.ended_by_vehicle.is_some()
            || status.mission_state == Some(MissionState::Failed),
        ended_because: status.ended_by_vehicle,
        holding: status.holding,
    }
}

#[cfg(test)]
mod tests;
