//! Session orchestration: build the host, launch the backend's stages,
//! the host, and the viewer in order — each gated on its readiness
//! signal — print the pinned URL, supervise, and tear everything down
//! in reverse order on ctrl-c or when any stage dies.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::backend::{SessionContext, SimBackend, Stage, backend_for};
use crate::cli::SimArgs;
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::ManagedChild;
use crate::readiness::{ReadySignal, await_ready};
use preflight::{build_host, prepare_web_assets};
pub(crate) use stages::{host_stage, viewer_stage};
use supervise::supervise;

pub(crate) mod fleet;
pub(crate) mod preflight;
mod stages;

/// How often the supervisor re-checks stage health.
const SUPERVISE_INTERVAL: Duration = Duration::from_millis(500);

/// How many times a restartable stage may die and be relaunched before
/// the session gives up (a crash loop is a failure, not a lifecycle).
const MAX_STAGE_RESTARTS: u32 = 3;

/// Runs one full SITL session until ctrl-c or a stage failure.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] for unknown backends, stale sessions,
/// missing artifacts, spawn/readiness failures, and stages that die
/// while the session runs.
pub async fn run_sim(args: &SimArgs) -> Result<(), XtaskError> {
    let backend = backend_for(&args.fc)?;
    let repo_root = repo_root()?;
    let mut ctx = sim_context(args, &repo_root, backend.as_ref())?;

    print_line(&format!(
        "launching a {} session (profile: {})",
        backend.name(),
        args.profile.as_env_value()
    ));
    print_line("building session host (release)...");
    build_host(&repo_root)?;
    // Out-of-the-box preflight: the viewer's generated wasm runtime is
    // gitignored, so a fresh checkout has none and the viewer loads dead.
    // Build it before serving. Backend-specific gitignored artifacts (the
    // px4 camera sidecar) are the backend's own best-effort `prepare`.
    prepare_web_assets(&repo_root)?;
    private_web_root_unless_primary(&mut ctx)?;
    backend.prepare(&ctx)?;

    let mut stages = backend.plan(&ctx)?;
    stages.push(host_stage(&ctx, backend.as_ref())?);
    if ctx.sim_camera {
        print_line("building the maplibre camera (release)...");
        crate::camera::build(&repo_root)?;
        // A single session spawns where its world puts the vehicle, which
        // xtask does not know, so its tiles load as the cameras ask.
        stages.push(crate::camera::stage(&ctx, None)?);
    }
    let xtask = std::env::current_exe().map_err(|source| XtaskError::Io {
        context: "locating the xtask executable for the viewer stage",
        source,
    })?;
    stages.push(viewer_stage(&ctx, &xtask)?);

    // The cancellation source must exist before the first child spawns:
    // each child lives in its own process group, so a terminal ctrl-c
    // delivered under default SIGINT disposition would kill only this
    // launcher and orphan every already-started stage.
    let mut cancel = spawn_cancel_listener()?;

    let (mut children, listening) = start_stages(&stages, &mut cancel).await?;

    let Some((actual_port, certificate)) = listening else {
        teardown(&mut children);
        return Err(XtaskError::Io {
            context: "the host never proved a listening port",
            source: std::io::Error::other("no LISTENING line was captured"),
        });
    };
    if let Err(error) = verify_listening_port(args.host_port, actual_port) {
        teardown(&mut children);
        return Err(error);
    }

    // The reset script consults this marker: while a supervisor owns
    // the flight controller, the script must not respawn its own.
    // Every failure past this point owns the already-running children:
    // returning without teardown would orphan the whole simulator stack.
    let pid_file = supervisor_marker(&ctx.repo_root, ctx.instance);
    claim_supervisor(&pid_file, &mut children)?;

    // Held for the session's lifetime: dropping it (any exit path from
    // here) unregisters the `pilotage.local` alias with the session.
    let _mdns_alias = announce::publish_connect_facts(args, &ctx, actual_port, &certificate);

    let outcome = supervise(&mut children, &stages, backend.as_ref(), &ctx, &mut cancel).await;
    std::fs::remove_file(&pid_file).ok();
    // The manifest describes a session that no longer exists past this
    // point; leaving it behind would let a later failed connect clobber
    // hand-typed connect inputs with a dead target.
    std::fs::remove_file(ctx.web_root.join(stages::MANIFEST_NAME)).ok();
    teardown(&mut children);
    outcome
}

/// The context of a `sim` session, once nothing else holds what it claims
/// and its run directory is ready.
fn sim_context(
    args: &SimArgs,
    repo_root: &Path,
    backend: &dyn SimBackend,
) -> Result<SessionContext, XtaskError> {
    let run_dir = repo_root.join(sim_run_dir(args.instance));
    let ctx = SessionContext {
        repo_root: repo_root.to_path_buf(),
        host_port: args.host_port,
        viewer_port: args.viewer_port,
        profile: args.profile,
        log_dir: run_dir.clone(),
        web_root: stages::viewer_source(repo_root),
        lan: args.lan,
        join_token_file: args.join_token_file.clone(),
        instance: args.instance,
        ready_timeout_s: args.ready_timeout_s,
        sim_camera: args.sim_camera,
    };
    // Before the run directory is touched: archiving logs out from under
    // a session that already runs there would lose its incident record.
    crate::conflicts::refuse_conflicts(&crate::conflicts::session_claims(&ctx, backend))?;
    prepared_log_dir(&run_dir)?;
    Ok(ctx)
}

/// Gives a session other than instance 0 a web root of its own; instance
/// 0 owns the shared `clients/web/session.json`.
///
/// Called once the viewer's generated runtime exists: the web root links
/// the viewer tree's entries as they are when it is built, so a root
/// built first would serve no runtime and the viewer would load dead.
fn private_web_root_unless_primary(ctx: &mut SessionContext) -> Result<(), XtaskError> {
    if ctx.instance != 0 {
        ctx.web_root = stages::private_web_root(&ctx.repo_root, &ctx.log_dir)?;
    }
    Ok(())
}

/// Registers the SIGINT handler and returns a receiver that flips to
/// `true` on ctrl-c. Registration happens synchronously in this call so
/// no child can be spawned while the default (kill-the-launcher-only)
/// disposition is still active.
fn spawn_cancel_listener() -> Result<tokio::sync::watch::Receiver<bool>, XtaskError> {
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|source| XtaskError::Io {
            context: "registering the ctrl-c handler",
            source,
        })?;
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        if sigint.recv().await.is_some() {
            tx.send(true).ok();
        }
    });
    Ok(rx)
}

/// Fails closed when the host proves a different port than the one
/// requested: the URL printed to the user must be a port the host
/// actually bound.
fn verify_listening_port(requested: u16, actual: u16) -> Result<(), XtaskError> {
    if actual == requested {
        Ok(())
    } else {
        Err(XtaskError::PortMismatch { requested, actual })
    }
}

/// Writes the supervisor pid marker, tearing the session down when the
/// write fails: a session the reset script cannot coordinate with must
/// not keep running.
fn claim_supervisor(
    pid_file: &std::path::Path,
    children: &mut Vec<ManagedChild>,
) -> Result<(), XtaskError> {
    if let Err(source) = std::fs::write(pid_file, std::process::id().to_string()) {
        teardown(children);
        return Err(XtaskError::Io {
            context: "writing the supervisor pid marker",
            source,
        });
    }
    Ok(())
}

/// Spawns every stage in order, awaiting each one's readiness signal or
/// a ctrl-c. Already-started children — including the child whose
/// readiness wait a cancellation interrupts — are torn down on any exit
/// but success.
async fn start_stages(
    stages: &[Stage],
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(Vec<ManagedChild>, Option<(u16, String)>), XtaskError> {
    let mut children: Vec<ManagedChild> = Vec::new();
    let mut listening = None;
    for stage in stages {
        print_line(&format!("starting {}...", stage.spec.name));
        let mut child = match ManagedChild::spawn(&stage.spec) {
            Ok(child) => child,
            Err(error) => {
                teardown(&mut children);
                return Err(error);
            }
        };
        let ready = tokio::select! {
            ready = await_ready(&mut child, &stage.readiness) => ready,
            _ = cancel.changed() => {
                print_line("");
                print_line("stopping the session...");
                child.terminate_group();
                teardown(&mut children);
                return Err(XtaskError::Cancelled);
            }
        };
        match ready {
            Ok(ReadySignal::HostListening { port, certificate }) => {
                listening = Some((port, certificate));
            }
            Ok(ReadySignal::Up) => {}
            Err(error) => {
                child.terminate_group();
                teardown(&mut children);
                return Err(error);
            }
        }
        print_line(&format!(
            "{} ready (log: {})",
            stage.spec.name,
            child.log_path.display()
        ));
        children.push(child);
    }
    Ok((children, listening))
}

/// Resets the running simulation via the selected backend.
///
/// # Errors
///
/// Returns the backend's typed reset failure.
pub fn run_reset(fc: &str, instance: u16) -> Result<(), XtaskError> {
    let backend = backend_for(fc)?;
    backend.reset_instance(&repo_root()?, instance)
}

/// The supervisor marker's file name inside a session's run directory.
const SUPERVISOR_MARKER: &str = "supervisor.pid";

/// Where the supervisor of simulator instance `instance` records its pid.
///
/// One place per instance, whichever command supervises it, so a reset
/// finds a fleet vehicle's supervisor the same way as a `sim` session's.
/// Instance 0's is `target/xtask-sim/supervisor.pid`, which the reset
/// scripts read.
pub(crate) fn supervisor_marker(repo_root: &Path, instance: u16) -> PathBuf {
    repo_root
        .join(sim_run_dir(instance))
        .join(SUPERVISOR_MARKER)
}

/// The session working directory, relative to the repository root.
/// Stage logs and the standalone handshake default land in one place.
pub(crate) const SESSION_DIR: &str = "target/xtask-sim";

/// The run directory of a `sim` session, relative to the repository root.
///
/// Instance 0 keeps [`SESSION_DIR`] itself: the reset scripts find the
/// supervisor's pid marker there. Another instance gets a directory of
/// its own beneath it, so concurrent sessions never archive or overwrite
/// one another's logs.
pub(crate) fn sim_run_dir(instance: u16) -> PathBuf {
    let dir = PathBuf::from(SESSION_DIR);
    if instance == 0 {
        dir
    } else {
        dir.join(format!("instance-{instance}"))
    }
}

/// Ensures the stage-log directory exists with the previous run's logs
/// preserved out of the way, announcing where they went.
fn prepared_log_dir(log_dir: &Path) -> Result<PathBuf, XtaskError> {
    let log_dir = log_dir.to_path_buf();
    std::fs::create_dir_all(&log_dir).map_err(|source| XtaskError::Io {
        context: "creating the session log directory",
        source,
    })?;
    if let Some(archive) = crate::log_archive::archive_previous_logs(&log_dir)? {
        print_line(&format!(
            "previous run's logs preserved at {}",
            archive.display()
        ));
    }
    Ok(log_dir)
}

/// Produces the Alia X-Plane runtime handshake and prints its path.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] when X-Plane is unreachable or the
/// handshake cannot be produced.
pub fn run_handshake(out_dir: &Path) -> Result<(), XtaskError> {
    let repo_root = repo_root()?;
    let out = if out_dir.is_absolute() {
        out_dir.to_path_buf()
    } else {
        repo_root.join(out_dir)
    };
    std::fs::create_dir_all(&out).map_err(|source| XtaskError::Io {
        context: "creating the handshake output directory",
        source,
    })?;
    let path = crate::backend::produce_xplane_handshake_blocking(&repo_root, &out)?;
    print_line(&path.display().to_string());
    Ok(())
}

/// This repository's root: the checkout `cargo xtask` was INVOKED from.
///
/// `cargo run` exports `CARGO_MANIFEST_DIR` into the child's runtime
/// environment, pointing at the invoking workspace's `tools/xtask` — so a
/// binary cached from another checkout (a worktree, a moved clone) still
/// operates on the repository the user is standing in. The compile-time
/// path is only the fallback for running the binary outside cargo.
pub(crate) fn repo_root() -> Result<PathBuf, XtaskError> {
    let runtime_manifest = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from);
    let manifest = resolve_manifest(
        runtime_manifest.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    );
    root_from(manifest)
}

fn resolve_manifest<'a>(
    runtime_manifest: Option<&'a Path>,
    compiled_manifest: &'a Path,
) -> &'a Path {
    runtime_manifest.unwrap_or(compiled_manifest)
}

/// The workspace root two levels above `tools/xtask`.
fn root_from(manifest: &std::path::Path) -> Result<PathBuf, XtaskError> {
    manifest
        .ancestors()
        .nth(2)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| XtaskError::Io {
            context: "resolving the repository root",
            source: std::io::Error::other("tools/xtask has no grandparent"),
        })
}

/// Waits for ctrl-c (clean stop) or a stage dying. A dead
/// flight-controller stage is RESTARTED in place (bounded): the reset
/// flow kills the FC by design (world reset + FC restart), and the
/// session must survive it. Every other stage's death ends the session.
/// The restart's readiness wait races the cancellation source, so a
/// ctrl-c during an FC restart stops the session promptly instead of
/// waiting out the replacement's readiness deadline.
/// Stops every started stage in reverse launch order.
fn teardown(children: &mut Vec<ManagedChild>) {
    while let Some(mut child) = children.pop() {
        print_line(&format!("stopping {}...", child.name));
        child.terminate_group();
    }
}

fn open_in_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(error) = Command::new(opener).arg(url).spawn() {
        tracing::warn!(%error, opener, "could not open the browser; use the printed URL");
    }
}

mod announce;
mod supervise;

#[cfg(test)]
mod tests;
