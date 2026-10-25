//! The host's mission lane (ADR-0052): one per vehicle, at the vehicle's
//! control endpoint. It answers client intents through the sans-IO
//! [`pilotage_mission::Lane`], runs each approved plan as its own mission
//! principal, counts the pictures each survey line acquires, asks the
//! judgment service about a sample of them, and publishes its status. It
//! keeps its run in a journal, so that a restart restores the run and
//! replays nothing.

mod config;
mod frame_tap;
mod journal;
mod judge;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pilotage_adapter_api::RawVideoFrame;
use pilotage_mission::{
    FrameAssessment, IntentContext, Lane, LaneEffect, MissionState, Requester, RunExecution,
    RunOrder, RunPlan, RunProgress, Steering,
};
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{error, info, warn};

use super::automation::{self, AutomationStatus, MissionPlan, RunControl};
use super::engine_actor::ToEngine;
use super::operations::{self, OperationHost, RunInputs};
use crate::error::HostError;
use journal::LaneJournal;

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

/// Spawns the lane, restored from its journal, and registers it with the
/// engine actor. Returns the sender the frame tap feeds and the lane's
/// task.
///
/// # Errors
/// [`HostError::LaneJournal`] for a journal that cannot be read or restored.
pub(crate) async fn spawn(
    engine_tx: &mpsc::Sender<ToEngine>,
    start: Instant,
    config: LaneRuntimeConfig,
) -> Result<(mpsc::Sender<LaneInbound>, JoinHandle<()>), HostError> {
    let task = LaneTask::restored(config, engine_tx.clone(), start).await?;
    let (sender, inbound) = mpsc::channel(INBOUND_QUEUE);
    let registered = engine_tx
        .send(ToEngine::MissionLane {
            sender: sender.clone(),
        })
        .await;
    if registered.is_err() {
        warn!("the engine actor is gone; the mission lane does not start");
    }
    let handle = tokio::spawn(async move {
        task.run(inbound).await;
        tracing::debug!(task = "mission-lane", "task exited");
    });
    Ok((sender, handle))
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
    /// Where the runs of plans of operations read and write.
    operations: OperationHost,
    /// Where the lane keeps its run.
    journal: LaneJournal,
    /// A run the lane started that waits for the journal to hold it.
    unrecorded: Option<Box<RunOrder>>,
}

impl LaneTask {
    /// The lane as its journal left it, with the operations a restart left
    /// uncertain asked about.
    async fn restored(
        config: LaneRuntimeConfig,
        engine: mpsc::Sender<ToEngine>,
        start: Instant,
    ) -> Result<Self, HostError> {
        let held = match config.journal.as_deref() {
            Some(path) => journal::read(path).await?,
            None => None,
        };
        let journal = LaneJournal::new(config.journal.clone(), held.clone());
        let lane = match (held, config.journal.as_deref()) {
            (Some(record), Some(path)) => Lane::restore(config.lane.clone(), &record)
                .map_err(|reason| journal::journal_error(path, &reason))?,
            _ => Lane::new(config.lane.clone()),
        };
        info!(lane = %config.lane.name, simulated = config.lane.simulated, judge = config.decision.is_some(), state = ?lane.state(), "mission lane ready");
        let mut task = Self::new(config, engine, start, lane, journal);
        task.reconcile().await;
        Ok(task)
    }

    fn new(
        config: LaneRuntimeConfig,
        engine: mpsc::Sender<ToEngine>,
        start: Instant,
        lane: Lane,
        journal: LaneJournal,
    ) -> Self {
        Self {
            judge: config.decision.map(|provider| {
                judge::Judge::new(provider, config.assess_every, config.judge_height_m)
            }),
            lane,
            engine,
            start,
            planar_pose_is_truth: config.planar_pose_is_truth,
            run: None,
            runs: 0,
            ids: 0,
            last_publish: None,
            steering: Steering::default(),
            operations: config.operations,
            journal,
            unrecorded: None,
        }
    }

    /// Asks the executors whether the operations a restart left uncertain
    /// happened.
    async fn reconcile(&mut self) {
        for query in self.lane.status_queries() {
            match operations::status(&self.operations, &query).await {
                Ok(found) => {
                    if let Err(reason) = self.lane.reconcile(&query.task_id, found) {
                        error!(operation = %query.operation_id, %reason, "status not taken");
                    }
                }
                Err(reason) => {
                    warn!(operation = %query.operation_id, %reason, "whether the operation happened is not known; it stays uncertain");
                }
            }
        }
        self.journal.keep(self.lane.record()).await;
    }

    /// Makes the journal hold the run as it is now, and starts the run that
    /// waited for it.
    async fn record(&mut self) {
        if self.journal.keep(self.lane.record()).await
            && let Some(order) = self.unrecorded.take()
        {
            self.apply(LaneEffect::Start(order)).await;
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
                    if let Some((ticket, judgment)) = result {
                        let assessment = judgment.map(|judgment| FrameAssessment {
                            usable: judgment.usable >= 0.5,
                            hazard: judgment.hazard >= 0.5,
                        });
                        self.lane.on_host_clock(self.host_ms());
                        if let Err(refused) = self.lane.on_assessment(&ticket, assessment) {
                            warn!(%refused, "judgment refused");
                        }
                    }
                },
                _ = ticker.tick() => {
                    self.lane.on_host_clock(self.host_ms());
                    if let Some(effect) = self.lane.on_clock(unix_now_ms()) {
                        self.apply(effect).await;
                    }
                    self.on_progress().await;
                    self.record().await;
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

    /// Milliseconds since the host started, on its monotonic clock.
    fn host_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
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
        self.record().await;
        self.publish(true).await
    }

    async fn apply(&mut self, effect: LaneEffect) {
        let control = match effect {
            LaneEffect::Start(order) => {
                // A run starts only once the journal holds it, so that a
                // restart finds every run that may have had an effect.
                if !self.journal.keep(self.lane.record()).await {
                    warn!(run = %order.run_id, "the run waits for the lane journal");
                    self.unrecorded = Some(order);
                    return;
                }
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
                // A hold the lane took while the run waited for the journal
                // reaches the run before its first operation.
                if matches!(
                    self.lane.state(),
                    wire::MissionLaneState::HoldRequested | wire::MissionLaneState::Held
                ) {
                    controls.try_send(RunControl::Hold).ok();
                }
                let order = *order;
                let (handle, status) = match order.plan {
                    RunPlan::Operations(plan) => operations::spawn_operation_run(
                        order.run_id.clone(),
                        plan,
                        self.operations.clone(),
                        receiver,
                        RunInputs {
                            results: order.results,
                            recorded: self.journal.ended(),
                        },
                    ),
                    plan => {
                        let Some(plan) = MissionPlan::of_lane(plan, order.config) else {
                            return;
                        };
                        automation::spawn_mission_run(
                            &self.engine,
                            self.start,
                            plan,
                            self.planar_pose_is_truth,
                            client,
                            Some(receiver),
                        )
                    }
                };
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
            LaneEffect::Abort => {
                // A run that never started needs no stop.
                self.unrecorded = None;
                RunControl::Abort
            }
            LaneEffect::Answer(answer) => RunControl::Answer(answer),
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
        if let Some(ticket) = self.lane.on_frame()
            && let Some(judge) = self.judge.as_mut()
            && judge.offer(ticket.clone(), frame, self.lane.wants_judgment(&ticket))
        {
            self.lane.on_sent(&ticket);
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
        active_task: status.active_task.clone(),
        finished_tasks: status.finished_tasks.clone(),
        plan_complete: status.plan_complete,
        ended: finished
            || status.fenced
            || status.closed
            || status.ended_by_vehicle.is_some()
            || status.ended_because.is_some()
            || status.mission_state == Some(MissionState::Failed),
        ended_because: status.ended_by_vehicle.or(status.ended_because),
        holding: status.holding,
        execution: Some(execution(status)),
        results: status.results.clone(),
        cancelled_tasks: status.cancelled_tasks.clone(),
        failed_tasks: status.failed_tasks.clone(),
        questions: status.questions.clone(),
    }
}

/// What the run does now, from its status.
fn execution(status: &AutomationStatus) -> RunExecution {
    if status.stopped {
        RunExecution::Stopped
    } else if status.awaiting_launch {
        RunExecution::AwaitingLaunch
    } else if status.held {
        RunExecution::LeaseHeldElsewhere(status.lease_holder)
    } else if status.operator_held {
        RunExecution::Holding
    } else {
        RunExecution::Flying
    }
}

#[cfg(test)]
mod tests;
