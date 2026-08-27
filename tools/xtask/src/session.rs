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
use crate::process::{ManagedChild, ProcessSpec};
use crate::readiness::{Readiness, ReadySignal, await_ready, stage_log};
use preflight::{build_host, prepare_web_assets};
use supervise::supervise;

pub(crate) mod preflight;

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
    let log_dir = prepared_log_dir(&repo_root)?;
    let ctx = SessionContext {
        repo_root: repo_root.clone(),
        host_port: args.host_port,
        viewer_port: args.viewer_port,
        profile: args.profile,
        log_dir: log_dir.clone(),
        lan: args.lan,
    };

    refuse_stale_session(backend.stale_process_patterns())?;

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
    backend.prepare(&ctx)?;

    let mut stages = backend.plan(&ctx)?;
    stages.push(host_stage(&ctx, backend.as_ref()));
    stages.push(viewer_stage(&ctx)?);

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
    let pid_file = log_dir.join("supervisor.pid");
    claim_supervisor(&pid_file, &mut children)?;

    // Held for the session's lifetime: dropping it (any exit path from
    // here) unregisters the `pilotage.local` alias with the session.
    let _mdns_alias = announce::publish_connect_facts(args, &ctx, actual_port, &certificate);

    let outcome = supervise(&mut children, &stages, backend.as_ref(), &ctx, &mut cancel).await;
    std::fs::remove_file(&pid_file).ok();
    // The manifest describes a session that no longer exists past this
    // point; leaving it behind would let a later failed connect clobber
    // hand-typed connect inputs with a dead target.
    std::fs::remove_file(ctx.repo_root.join("clients/web/session.json")).ok();
    teardown(&mut children);
    outcome
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
pub fn run_reset(fc: &str) -> Result<(), XtaskError> {
    let backend = backend_for(fc)?;
    backend.reset(&repo_root()?)
}

/// This repository's root: the checkout `cargo xtask` was INVOKED from.
///
/// `cargo run` exports `CARGO_MANIFEST_DIR` into the child's runtime
/// Ensures the stage-log directory exists with the previous run's logs
/// preserved out of the way, announcing where they went.
fn prepared_log_dir(repo_root: &std::path::Path) -> Result<PathBuf, XtaskError> {
    let log_dir = repo_root.join("target/xtask-sim");
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

/// Cargo sets `CARGO_MANIFEST_DIR` in the invoking process's
/// environment, pointing at the invoking workspace's `tools/xtask` — so a
/// binary cached from another checkout (a worktree, a moved clone) still
/// operates on the repository the user is standing in. The compile-time
/// path is only the fallback for running the binary outside cargo.
fn repo_root() -> Result<PathBuf, XtaskError> {
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

/// Refuses to start over another session's processes: the launcher only
/// ever kills processes it started.
fn refuse_stale_session(mut patterns: Vec<&'static str>) -> Result<(), XtaskError> {
    patterns.push("pilotage-session-host");
    let mut listing = String::new();
    for pattern in patterns {
        let Ok(output) = Command::new("pgrep").args(["-f", pattern]).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        for pid in String::from_utf8_lossy(&output.stdout).split_whitespace() {
            // `ps` prints the command line without the environment noise
            // macOS `pgrep -fl` appends.
            let Ok(line) = Command::new("ps")
                .args(["-o", "pid=,command=", "-p", pid])
                .output()
            else {
                continue;
            };
            listing.push_str(String::from_utf8_lossy(&line.stdout).trim_end());
            listing.push('\n');
        }
    }
    if listing.trim().is_empty() {
        Ok(())
    } else {
        Err(XtaskError::StaleSession {
            listing: listing.trim_end().to_owned(),
        })
    }
}

/// The host stage, with the environment DERIVED from the backend rather
/// than handed in.
///
/// Taking a ready-made environment is what let a caller assemble the
/// right one and then pass a different one — two correct halves and a
/// wrong join, which no test of either half can see. With no such
/// parameter there is nothing to get wrong at the call site.
pub(crate) fn host_stage(ctx: &SessionContext, backend: &dyn SimBackend) -> Stage {
    let adapter = backend.host_adapter();
    let mut env = crate::backend::host_env_for(backend, ctx);
    env.push((
        "PILOTAGE_AVIATE_PROFILE".to_owned(),
        ctx.profile.as_env_value().to_owned(),
    ));
    if std::env::var_os("RUST_LOG").is_none() {
        env.push(("RUST_LOG".to_owned(), "info".to_owned()));
    }
    Stage {
        spec: ProcessSpec {
            name: "session-host",
            program: ctx
                .repo_root
                .join("target/release/pilotage-session-host")
                .display()
                .to_string(),
            args: vec![
                "--port".to_owned(),
                ctx.host_port.to_string(),
                "--adapter".to_owned(),
                adapter.to_owned(),
            ],
            cwd: Some(ctx.repo_root.clone()),
            env,
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "session-host"),
        },
        readiness: Readiness::HostListening { timeout_s: 60 },
    }
}

/// The static viewer. The host learning `--serve-web` retires this
/// stage; until then python3 serves `clients/web` exactly like the
/// recorded runbook.
fn viewer_stage(ctx: &SessionContext) -> Result<Stage, XtaskError> {
    let viewer_dir = ctx.repo_root.join("clients/web");
    if !viewer_dir.join("index.html").is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "viewer entrypoint",
            path: viewer_dir.join("index.html"),
            hint: "run from the Pilotage repository root",
        });
    }
    // Cache-Control: no-store on every response: the served client must
    // always equal the working-tree source. The viewer is served straight
    // from the tree, so any heuristic caching of main.js would let the
    // browser run a client that has diverged from the source on disk;
    // no-store holds the served client and the on-disk source identical.
    let server = format!(
        "import http.server\n\
         class H(http.server.SimpleHTTPRequestHandler):\n\
         \tdef end_headers(self):\n\
         \t\tself.send_header('Cache-Control', 'no-store')\n\
         \t\tsuper().end_headers()\n\
         http.server.test(HandlerClass=H, port={port}, bind='{bind}')\n",
        port = ctx.viewer_port,
        // Loopback by default: serving the working tree to the network is
        // an explicit request, never a side effect of running a sim.
        bind = if ctx.lan { "0.0.0.0" } else { "127.0.0.1" }
    );
    Ok(Stage {
        spec: ProcessSpec {
            name: "viewer",
            program: "python3".to_owned(),
            args: vec!["-c".to_owned(), server],
            cwd: Some(viewer_dir),
            env: vec![],
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "viewer"),
        },
        readiness: Readiness::TcpAccepts {
            port: ctx.viewer_port,
            timeout_s: 15,
        },
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
