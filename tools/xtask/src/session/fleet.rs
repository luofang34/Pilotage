//! `cargo xtask fleet`: one session per vehicle (ADR-0028 roster attach),
//! each with its own simulator, host, and viewer, launched in order,
//! supervised together, and torn down together.

use std::path::{Path, PathBuf};

use super::preflight::{build_host, prepare_web_assets};
use super::stages::{self, MANIFEST_NAME};
use super::supervise::{Watched, check_once};
use super::{
    SUPERVISE_INTERVAL, prepared_log_dir, repo_root, spawn_cancel_listener, start_stages, teardown,
    verify_listening_port,
};
use crate::backend::{SessionContext, SimBackend, SpawnPoint, Stage, backend_for};
use crate::cli::FleetArgs;
use crate::conflicts::{Claim, refuse_conflicts, session_claims};
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::ManagedChild;
use crate::readiness::{session_manifest, viewer_url};

/// The fleet's working directory, relative to the repository root. Each
/// vehicle runs in `<FLEET_DIR>/<name>/`, and the fleet manifest sits
/// beside them.
pub(crate) const FLEET_DIR: &str = "target/xtask-fleet";
/// The fleet manifest's file name inside [`FLEET_DIR`].
const FLEET_MANIFEST: &str = "fleet.json";
/// Where the first vehicle spawns: PX4's Zurich default home.
pub(crate) const FLEET_ORIGIN: SpawnPoint = SpawnPoint {
    latitude_deg: 47.397742,
    longitude_deg: 8.545594,
};
/// How far apart the vehicles spawn, east of one another. Far enough to
/// be visibly apart on the map, near enough to share one view.
const SPACING_M: f64 = 30.0;
/// Metres per degree of latitude (and of longitude at the equator).
const METRES_PER_DEGREE: f64 = 111_320.0;
/// The address every fleet host is reached at: fleets serve loopback.
const LOOPBACK: &str = "127.0.0.1";

/// Where vehicle `index` spawns: `index * SPACING_M` east of the origin.
pub(crate) fn spawn_point(index: usize) -> SpawnPoint {
    let east_m = SPACING_M * index as f64;
    let metres_per_lon_degree = METRES_PER_DEGREE * FLEET_ORIGIN.latitude_deg.to_radians().cos();
    SpawnPoint {
        latitude_deg: FLEET_ORIGIN.latitude_deg,
        longitude_deg: FLEET_ORIGIN.longitude_deg + east_m / metres_per_lon_degree,
    }
}

/// A vehicle's session name: its spec with the model joined by `-`, and
/// its 1-based position, so two vehicles of one kind stay apart.
pub(crate) fn session_name(spec: &str, index: usize) -> String {
    format!("{}-{}", spec.replace(':', "-"), index + 1)
}

/// One vehicle, planned and ready to launch.
struct Slot {
    name: String,
    spec: String,
    backend: Box<dyn SimBackend>,
    ctx: SessionContext,
    stages: Vec<Stage>,
}

/// One vehicle whose stages are running.
struct Running {
    slot: Slot,
    children: Vec<ManagedChild>,
    port: u16,
    certificate: String,
    fc_restarts: u32,
}

/// One entry of the fleet manifest: the connect facts of
/// `clients/web/session.json`, plus which vehicle it is.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FleetEntry {
    /// The session name.
    pub(crate) name: String,
    /// The `<backend>[:<model>]` the vehicle runs.
    pub(crate) backend: String,
    /// The host a client dials.
    pub(crate) host: String,
    /// The host's WebTransport port.
    pub(crate) port: u16,
    /// The host certificate's SHA-256, hex.
    pub(crate) cert_hash: String,
}

/// The fleet manifest document.
#[derive(Debug, serde::Serialize)]
pub(crate) struct FleetManifest {
    /// Every vehicle, in launch order.
    pub(crate) sessions: Vec<FleetEntry>,
}

/// Refuses a fleet whose vehicles claim the same thing, naming both.
/// Checked before probing the machine: such a fleet conflicts with
/// itself, whatever else runs.
///
/// # Errors
///
/// Returns [`XtaskError::FleetClaimOverlap`] for the first shared claim.
pub(crate) fn refuse_overlapping_claims(slots: &[(String, Vec<Claim>)]) -> Result<(), XtaskError> {
    for (later, (second, claims)) in slots.iter().enumerate() {
        for (first, earlier) in slots.iter().take(later) {
            if let Some(claim) = claims.iter().find(|claim| earlier.contains(claim)) {
                return Err(XtaskError::FleetClaimOverlap {
                    first: first.clone(),
                    second: second.clone(),
                    claim: format!("{claim:?}"),
                });
            }
        }
    }
    Ok(())
}

/// Runs a fleet until ctrl-c or any session's failure.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] for unknown backends or models, claims
/// that overlap or are held, a backend that cannot place its vehicle,
/// and any session's launch or runtime failure. Every started session is
/// torn down on every exit.
pub async fn run_fleet(args: &FleetArgs) -> Result<(), XtaskError> {
    let repo_root = repo_root()?;
    let mut slots = resolve_slots(args, &repo_root)?;
    let claims: Vec<(String, Vec<Claim>)> = slots
        .iter()
        .map(|slot| {
            let claims = session_claims(&slot.ctx, slot.backend.as_ref());
            (slot.name.clone(), claims)
        })
        .collect();
    refuse_overlapping_claims(&claims)?;
    let every_claim: Vec<Claim> = claims.into_iter().flat_map(|(_, claims)| claims).collect();
    refuse_conflicts(&every_claim)?;

    for slot in &slots {
        prepared_log_dir(&slot.ctx.log_dir)?;
    }
    print_line(&format!("launching a fleet of {} vehicles", slots.len()));
    print_line("building session host (release)...");
    build_host(&repo_root)?;
    prepare_web_assets(&repo_root)?;
    // After the viewer's runtime is built: a private web root links the
    // viewer tree's entries as they are now. The first vehicle owns
    // `clients/web/session.json`, so a viewer opened the single-session
    // way still finds a live session.
    for slot in slots.iter_mut().skip(1) {
        slot.ctx.web_root = stages::private_web_root(&repo_root, &slot.ctx.log_dir)?;
    }
    for (index, slot) in slots.iter_mut().enumerate() {
        plan_slot(slot, index)?;
    }

    let mut cancel = spawn_cancel_listener()?;
    let mut running: Vec<Running> = Vec::new();
    for slot in slots {
        match launch_slot(slot, &mut cancel).await {
            Ok(session) => running.push(session),
            Err(error) => {
                stop_all(&mut running);
                return Err(error);
            }
        }
    }
    let manifest_path = repo_root.join(FLEET_DIR).join(FLEET_MANIFEST);
    write_fleet_manifest(&manifest_path, &running);
    if args.open {
        for session in &running {
            super::open_in_browser(&session_url(session));
        }
    }
    print_line("press ctrl-c to stop the fleet");

    let outcome = supervise_all(&mut running, &mut cancel).await;
    std::fs::remove_file(&manifest_path).ok();
    stop_all(&mut running);
    outcome
}

/// Resolves every vehicle's backend and context, fail-closed, before
/// anything on the machine is touched.
fn resolve_slots(args: &FleetArgs, repo_root: &Path) -> Result<Vec<Slot>, XtaskError> {
    let mut slots = Vec::with_capacity(args.vehicles.len());
    for (index, spec) in args.vehicles.iter().enumerate() {
        let offset = u16::try_from(index).map_err(|_| XtaskError::Usage {
            message: format!("a fleet holds at most {} vehicles", u16::MAX),
        })?;
        let backend = backend_for(spec)?;
        let name = session_name(spec, index);
        let ctx = SessionContext {
            repo_root: repo_root.to_path_buf(),
            host_port: args.host_port.saturating_add(offset),
            viewer_port: args.viewer_port.saturating_add(offset),
            profile: args.profile,
            log_dir: repo_root.join(FLEET_DIR).join(&name),
            web_root: stages::viewer_source(repo_root),
            lan: false,
            instance: args.instance_base.saturating_add(offset),
        };
        slots.push(Slot {
            name,
            spec: spec.clone(),
            backend,
            ctx,
            stages: Vec::new(),
        });
    }
    Ok(slots)
}

/// Plans one vehicle's stages at its spawn point, host and viewer last.
fn plan_slot(slot: &mut Slot, index: usize) -> Result<(), XtaskError> {
    slot.backend.prepare(&slot.ctx)?;
    let mut planned = slot.backend.plan(&slot.ctx)?;
    slot.backend
        .spawn(&slot.ctx, &spawn_point(index), &mut planned)?;
    planned.push(stages::host_stage(&slot.ctx, slot.backend.as_ref()));
    let xtask = std::env::current_exe().map_err(|source| XtaskError::Io {
        context: "locating the xtask executable for the viewer stage",
        source,
    })?;
    planned.push(stages::viewer_stage(&slot.ctx, &xtask)?);
    slot.stages = planned;
    Ok(())
}

/// Starts one vehicle's stages and publishes its connect manifest.
async fn launch_slot(
    slot: Slot,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Running, XtaskError> {
    print_line(&format!(
        "starting vehicle {} ({})...",
        slot.name, slot.spec
    ));
    let (mut children, listening) = start_stages(&slot.stages, cancel).await?;
    let Some((port, certificate)) = listening else {
        teardown(&mut children);
        return Err(XtaskError::Io {
            context: "the host never proved a listening port",
            source: std::io::Error::other("no LISTENING line was captured"),
        });
    };
    if let Err(error) = verify_listening_port(slot.ctx.host_port, port) {
        teardown(&mut children);
        return Err(error);
    }
    let manifest = slot.ctx.web_root.join(MANIFEST_NAME);
    if let Err(error) = std::fs::write(
        &manifest,
        session_manifest(LOOPBACK, None, port, &certificate),
    ) {
        print_line(&format!(
            "warning: could not write {}: {error}",
            manifest.display()
        ));
    }
    // The reset finds a vehicle's supervisor by its instance; this fleet
    // is that supervisor.
    let marker = super::supervisor_marker(&slot.ctx.repo_root, slot.ctx.instance);
    let claimed = marker
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&marker, std::process::id().to_string()));
    if let Err(source) = claimed {
        teardown(&mut children);
        return Err(XtaskError::Io {
            context: "writing the supervisor pid marker",
            source,
        });
    }
    let session = Running {
        slot,
        children,
        port,
        certificate,
        fc_restarts: 0,
    };
    print_line(&format!(
        "vehicle {} ready: {}",
        session.slot.name,
        session_url(&session)
    ));
    Ok(session)
}

fn session_url(session: &Running) -> String {
    viewer_url(
        LOOPBACK,
        session.slot.ctx.viewer_port,
        session.port,
        &session.certificate,
    )
}

/// The fleet manifest for the running sessions, as JSON.
fn fleet_manifest_json(running: &[Running]) -> Result<String, serde_json::Error> {
    let manifest = FleetManifest {
        sessions: running
            .iter()
            .map(|session| FleetEntry {
                name: session.slot.name.clone(),
                backend: session.slot.spec.clone(),
                host: LOOPBACK.to_owned(),
                port: session.port,
                cert_hash: session.certificate.clone(),
            })
            .collect(),
    };
    serde_json::to_string_pretty(&manifest)
}

/// Writes the fleet manifest. Best-effort, like each session's own: a
/// failed write loses discovery, never a running vehicle.
fn write_fleet_manifest(path: &Path, running: &[Running]) {
    let written = fleet_manifest_json(running)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(path, json + "\n"));
    match written {
        Ok(()) => print_line(&format!("fleet manifest: {}", path.display())),
        Err(error) => print_line(&format!(
            "warning: could not write {}: {error}",
            path.display()
        )),
    }
}

/// Watches every session until ctrl-c or any session's failure.
async fn supervise_all(
    running: &mut [Running],
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), XtaskError> {
    loop {
        tokio::select! {
            _ = cancel.changed() => {
                print_line("");
                print_line("stopping the fleet...");
                return Ok(());
            }
            () = tokio::time::sleep(SUPERVISE_INTERVAL) => {
                for session in running.iter_mut() {
                    let mut watched = Watched {
                        children: &mut session.children,
                        stages: &session.slot.stages,
                        backend: session.slot.backend.as_ref(),
                        ctx: &session.slot.ctx,
                        fc_restarts: &mut session.fc_restarts,
                    };
                    match check_once(&mut watched, cancel).await {
                        Ok(true) => {}
                        Ok(false) => return Ok(()),
                        Err(error) => {
                            print_line(&format!("vehicle {} failed", session.slot.name));
                            return Err(error);
                        }
                    }
                }
            }
        }
    }
}

/// Stops every running session, the last launched first, and removes
/// each session's connect manifest: it names a session that is gone.
fn stop_all(running: &mut Vec<Running>) {
    while let Some(mut session) = running.pop() {
        print_line(&format!("stopping vehicle {}...", session.slot.name));
        std::fs::remove_file(manifest_of(&session.slot.ctx)).ok();
        std::fs::remove_file(super::supervisor_marker(
            &session.slot.ctx.repo_root,
            session.slot.ctx.instance,
        ))
        .ok();
        teardown(&mut session.children);
    }
}

fn manifest_of(ctx: &SessionContext) -> PathBuf {
    ctx.web_root.join(MANIFEST_NAME)
}

#[cfg(test)]
mod tests;
