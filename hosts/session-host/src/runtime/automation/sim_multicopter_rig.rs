//! The in-process mission rig over the simulated multicopter: the engine
//! actor and the mission principal wired as the host runtime wires them
//! for `--adapter sim-multicopter`, with no transport endpoint.

use pilotage_adapter_sim_multicopter::{SimConfig, SimMulticopterAdapter};
use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{
    AutomationStatus, MissionRig, OBSERVER_CLIENT, RunControl, TelemetryObserver, prepare,
    spawn_mission_run, spawn_mission_task,
};
use crate::error::HostError;
use crate::runtime::engine_actor::ToEngine;
use crate::runtime::engine_actor::{ENGINE_QUEUE_CAPACITY, EngineActor};
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;
use crate::runtime::{HOST_VEHICLE, MissionOptions, RuntimeOptions, build_engine};

/// Spawns [`MissionRig`] over the simulated multicopter at `vehicle`'s
/// launch point, flying `options`. The camera does not start, so the rig
/// needs no network.
///
/// # Errors
///
/// [`HostError`] when the mission does not plan.
pub fn spawn_sim_multicopter_rig(
    vehicle: SimConfig,
    options: MissionOptions,
) -> Result<MissionRig, HostError> {
    let plan = prepare(&options)?;
    let start = Instant::now();
    let adapter = SimMulticopterAdapter::new(HOST_VEHICLE, vehicle);
    let engine = build_engine(&adapter, RuntimeOptions::default());
    let (engine_tx, engine_rx) = mpsc::channel(ENGINE_QUEUE_CAPACITY);
    let actor = tokio::spawn(EngineActor::new(engine, adapter, start).run(engine_rx));
    let (automation, status) = spawn_mission_task(&engine_tx, start, plan, false);
    Ok(MissionRig {
        engine_tx,
        status,
        actor,
        start,
        automation,
    })
}

/// The first client key a run takes; each later run takes the key below.
const FIRST_RUN_CLIENT: u64 = u64::MAX - 64;

/// Successive mission runs over one simulated multicopter, each its own
/// principal steered by its own controls, as a mission lane flies them:
/// the next run starts after the previous one stopped and released the
/// vehicle.
pub struct SurveyRuns {
    engine_tx: mpsc::Sender<ToEngine>,
    actor: JoinHandle<()>,
    start: Instant,
    runs: u64,
    active: Option<(JoinHandle<()>, mpsc::Sender<RunControl>)>,
}

/// Spawns [`SurveyRuns`] over the simulated multicopter at `vehicle`'s
/// launch point, with no run yet.
#[must_use]
pub fn spawn_sim_multicopter_runs(vehicle: SimConfig) -> SurveyRuns {
    let start = Instant::now();
    let adapter = SimMulticopterAdapter::new(HOST_VEHICLE, vehicle);
    let engine = build_engine(&adapter, RuntimeOptions::default());
    let (engine_tx, engine_rx) = mpsc::channel(ENGINE_QUEUE_CAPACITY);
    let actor = tokio::spawn(EngineActor::new(engine, adapter, start).run(engine_rx));
    SurveyRuns {
        engine_tx,
        actor,
        start,
        runs: 0,
        active: None,
    }
}

impl SurveyRuns {
    /// Aborts the active run, waits for it to release the vehicle, and
    /// starts a run flying `options`. Returns the new run's status.
    ///
    /// # Errors
    ///
    /// [`HostError`] when the mission does not plan.
    pub async fn fly(
        &mut self,
        options: &MissionOptions,
    ) -> Result<watch::Receiver<AutomationStatus>, HostError> {
        let plan = prepare(options)?;
        if let Some((handle, controls)) = self.active.take() {
            controls.send(RunControl::Abort).await.ok();
            handle.await.ok();
        }
        let client = ClientKey::new(FIRST_RUN_CLIENT.wrapping_sub(self.runs));
        self.runs = self.runs.wrapping_add(1);
        let (controls, receiver) = mpsc::channel(4);
        let (handle, status) = spawn_mission_run(
            &self.engine_tx,
            self.start,
            plan,
            false,
            client,
            Some(receiver),
        );
        self.active = Some((handle, controls));
        Ok(status)
    }

    /// Registers an observing client and hands back its datagrams.
    pub async fn observe(&self) -> TelemetryObserver {
        let (outbound_tx, outbound) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
        self.engine_tx
            .send(ToEngine::ClientConnected {
                client: OBSERVER_CLIENT,
                sender: outbound_tx,
            })
            .await
            .ok();
        TelemetryObserver { outbound }
    }

    /// Ends the active run and the engine actor.
    pub async fn shutdown(mut self) {
        if let Some((handle, controls)) = self.active.take() {
            controls.send(RunControl::Abort).await.ok();
            handle.await.ok();
        }
        drop(self.engine_tx);
        self.actor.await.ok();
    }
}
