//! The mission principal's task: the host core's principal
//! ([`pilotage_host_core::automation::MissionTask`]) driven by tokio. The
//! task gives the principal the host clock, carries its commands to the
//! engine actor and the actor's replies back, steps it at the mission
//! cadence, applies the mission lane's run controls, and publishes its
//! status on a watch channel.

use pilotage_host_core::automation::{
    AutomationStatus, MissionPlan, MissionTask as Principal, RunControl, TICK_INTERVAL,
};
use pilotage_host_core::engine_actor::ToEngine as CoreCommand;
use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::runtime::connection::ToConnection;
use crate::runtime::engine_actor::ToEngine;

/// The in-process mission principal's task state.
pub(super) struct MissionTask {
    principal: Principal,
    /// The task holds only a weak handle to the engine actor: it can never
    /// keep the actor's command channel open on its own.
    engine: mpsc::WeakSender<ToEngine>,
    start: Instant,
    status: watch::Sender<AutomationStatus>,
    /// Hold, resume and abort from the mission lane.
    controls: Option<mpsc::Receiver<RunControl>>,
    /// The principal's outbound sender, registered with the actor when
    /// the principal connects.
    outbound: Option<mpsc::Sender<ToConnection>>,
}

impl MissionTask {
    pub(super) fn new(
        engine: mpsc::WeakSender<ToEngine>,
        start: Instant,
        plan: MissionPlan,
        status: watch::Sender<AutomationStatus>,
        planar_pose_is_truth: bool,
    ) -> Self {
        Self {
            principal: Principal::new(plan, planar_pose_is_truth),
            engine,
            start,
            status,
            controls: None,
            outbound: None,
        }
    }

    /// Binds the run to its lane: its principal key and its controls.
    pub(super) fn bind(&mut self, client: ClientKey, controls: Option<mpsc::Receiver<RunControl>>) {
        self.principal.bind_client(client);
        self.controls = controls;
    }

    /// Registers with the engine actor, opens the session, and services
    /// engine replies, run controls and the mission tick until the actor
    /// goes away or the principal stops.
    pub(super) async fn run(
        mut self,
        outbound_tx: mpsc::Sender<ToConnection>,
        mut outbound_rx: mpsc::Receiver<ToConnection>,
    ) {
        self.outbound = Some(outbound_tx);
        self.set_now();
        let started = self.principal.start();
        if !self.flush().await || !started {
            return;
        }
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut controls = self.controls.take();
        loop {
            let alive = tokio::select! {
                message = outbound_rx.recv() => match message {
                    Some(message) => {
                        self.set_now();
                        self.principal.on_connection_message(message)
                    }
                    None => false,
                },
                control = next_control(&mut controls) => {
                    self.set_now();
                    self.principal.on_control(control);
                    true
                }
                _ = ticker.tick() => {
                    self.set_now();
                    self.principal.on_tick()
                }
            };
            let delivered = self.flush().await;
            if !alive || !delivered {
                break;
            }
        }
        self.set_now();
        self.principal.finish();
        self.flush().await;
    }

    fn set_now(&mut self) {
        let nanos = u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.principal.set_now(nanos);
    }

    /// Sends the principal's commands to the engine actor and publishes
    /// its status; `false` means the actor is gone and the task should
    /// exit.
    async fn flush(&mut self) -> bool {
        let status = self.principal.status().clone();
        self.status.send_if_modified(|published| {
            let changed = *published != status;
            *published = status;
            changed
        });
        for command in self.principal.take_outbox() {
            let command = match command {
                CoreCommand::ClientConnected { client } => {
                    let Some(sender) = self.outbound.clone() else {
                        return false;
                    };
                    ToEngine::ClientConnected { client, sender }
                }
                CoreCommand::ClientMessage {
                    client,
                    message,
                    now,
                } => ToEngine::ClientMessage {
                    client,
                    message,
                    now,
                },
                CoreCommand::NavGuidance { vehicle, state } => {
                    ToEngine::NavGuidance { vehicle, state }
                }
            };
            let Some(engine) = self.engine.upgrade() else {
                return false;
            };
            if engine.send(command).await.is_err() {
                return false;
            }
        }
        true
    }
}

/// The next control, or never when the run has no lane.
async fn next_control(controls: &mut Option<mpsc::Receiver<RunControl>>) -> RunControl {
    match controls.as_mut() {
        Some(receiver) => match receiver.recv().await {
            Some(control) => control,
            // The lane is gone: nothing steers this run any more, so it
            // stops the way an abort does.
            None => {
                *controls = None;
                RunControl::Abort
            }
        },
        None => std::future::pending().await,
    }
}
