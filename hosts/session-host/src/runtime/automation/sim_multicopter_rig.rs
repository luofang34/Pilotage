//! The in-process mission rig over the simulated multicopter: the engine
//! actor and the mission principal wired as the host runtime wires them
//! for `--adapter sim-multicopter`, with no transport endpoint.

use pilotage_adapter_sim_multicopter::{SimConfig, SimMulticopterAdapter};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::{MissionRig, prepare, spawn_mission_task};
use crate::error::HostError;
use crate::runtime::engine_actor::{ENGINE_QUEUE_CAPACITY, EngineActor};
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
