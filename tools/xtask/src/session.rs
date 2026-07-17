//! Session orchestration: build the host, launch the backend's stages,
//! the host, and the viewer in order — each gated on its readiness
//! signal — print the pinned URL, supervise, and tear everything down
//! in reverse order on ctrl-c or when any stage dies.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::backend::{SessionContext, Stage, backend_for};
use crate::cli::SimArgs;
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::{ManagedChild, ProcessSpec};
use crate::readiness::{Readiness, ReadySignal, await_ready, stage_log, viewer_url};

/// How often the supervisor re-checks stage health.
const SUPERVISE_INTERVAL: Duration = Duration::from_millis(500);

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
    let log_dir = repo_root.join("target/xtask-sim");
    std::fs::create_dir_all(&log_dir).map_err(|source| XtaskError::Io {
        context: "creating the session log directory",
        source,
    })?;
    let ctx = SessionContext {
        repo_root: repo_root.clone(),
        host_port: args.host_port,
        viewer_port: args.viewer_port,
        profile: args.profile,
        log_dir: log_dir.clone(),
    };

    refuse_stale_session(backend.stale_process_patterns())?;

    print_line(&format!(
        "launching a {} session (profile: {})",
        backend.name(),
        args.profile.as_env_value()
    ));
    print_line("building session host (release)...");
    build_host(&repo_root)?;

    let mut stages = backend.plan(&ctx)?;
    stages.push(host_stage(
        &ctx,
        backend.host_adapter(),
        backend.host_env(&ctx),
    ));
    stages.push(viewer_stage(&ctx)?);

    let mut children: Vec<ManagedChild> = Vec::new();
    let mut certificate = String::new();
    for stage in &stages {
        print_line(&format!("starting {}...", stage.spec.name));
        let mut child = match ManagedChild::spawn(&stage.spec) {
            Ok(child) => child,
            Err(error) => {
                teardown(&mut children);
                return Err(error);
            }
        };
        match await_ready(&mut child, &stage.readiness).await {
            Ok(ReadySignal::HostCertificate(cert)) => certificate = cert,
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

    let url = viewer_url(args.viewer_port, args.host_port, &certificate);
    print_line("");
    print_line(&format!("session ready: {url}"));
    print_line("press ctrl-c to stop the session");
    if args.open {
        open_in_browser(&url);
    }

    let outcome = supervise(&mut children).await;
    teardown(&mut children);
    outcome
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

/// This repository's root (the workspace this binary was built from).
fn repo_root() -> Result<PathBuf, XtaskError> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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

fn build_host(repo_root: &std::path::Path) -> Result<(), XtaskError> {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "pilotage-session-host"])
        .current_dir(repo_root)
        .status()
        .map_err(|source| XtaskError::Io {
            context: "building the session host",
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(XtaskError::CommandFailed {
            context: "cargo build --release -p pilotage-session-host",
            status: status.to_string(),
        })
    }
}

fn host_stage(ctx: &SessionContext, adapter: &str, mut env: Vec<(String, String)>) -> Stage {
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
    Ok(Stage {
        spec: ProcessSpec {
            name: "viewer",
            program: "python3".to_owned(),
            args: vec![
                "-m".to_owned(),
                "http.server".to_owned(),
                ctx.viewer_port.to_string(),
                "--bind".to_owned(),
                "127.0.0.1".to_owned(),
            ],
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

/// Waits for ctrl-c (clean stop) or any stage dying (error).
async fn supervise(children: &mut [ManagedChild]) -> Result<(), XtaskError> {
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|source| XtaskError::Io {
                    context: "waiting for ctrl-c",
                    source,
                })?;
                print_line("");
                print_line("stopping the session...");
                return Ok(());
            }
            () = tokio::time::sleep(SUPERVISE_INTERVAL) => {
                for child in children.iter_mut() {
                    child.check_running()?;
                }
            }
        }
    }
}

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
