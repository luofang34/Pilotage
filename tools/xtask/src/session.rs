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

    let (mut children, certificate) = start_stages(&stages).await?;

    // The reset script consults this marker: while a supervisor owns
    // the flight controller, the script must not respawn its own.
    // Every failure past this point owns the already-running children:
    // returning without teardown would orphan the whole simulator stack.
    let pid_file = log_dir.join("supervisor.pid");
    if let Err(source) = std::fs::write(&pid_file, std::process::id().to_string()) {
        teardown(&mut children);
        return Err(XtaskError::Io {
            context: "writing the supervisor pid marker",
            source,
        });
    }

    let url = viewer_url(args.viewer_port, args.host_port, &certificate);
    print_line("");
    print_line(&format!("session ready: {url}"));
    print_line("press ctrl-c to stop the session");
    if args.open {
        open_in_browser(&url);
    }

    let outcome = supervise(&mut children, &stages).await;
    std::fs::remove_file(&pid_file).ok();
    teardown(&mut children);
    outcome
}

/// Spawns every stage in order, awaiting each one's readiness signal.
/// Already-started children are torn down on any failure.
async fn start_stages(stages: &[Stage]) -> Result<(Vec<ManagedChild>, String), XtaskError> {
    let mut children: Vec<ManagedChild> = Vec::new();
    let mut certificate = String::new();
    for stage in stages {
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
    Ok((children, certificate))
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

/// Waits for ctrl-c (clean stop) or a stage dying. A dead
/// flight-controller stage is RESTARTED in place (bounded): the reset
/// flow kills the FC by design (world reset + FC restart), and the
/// session must survive it. Every other stage's death ends the session.
async fn supervise(children: &mut [ManagedChild], stages: &[Stage]) -> Result<(), XtaskError> {
    let mut fc_restarts: u32 = 0;
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
                for index in 0..children.len() {
                    let Err(death) = children[index].check_running() else {
                        continue;
                    };
                    let stage = &stages[index];
                    if stage.spec.name != "flight-controller" {
                        return Err(death);
                    }
                    fc_restarts = fc_restarts.wrapping_add(1);
                    if fc_restarts > MAX_STAGE_RESTARTS {
                        return Err(death);
                    }
                    print_line(&format!(
                        "flight-controller exited (reset or crash); restarting ({fc_restarts}/{MAX_STAGE_RESTARTS})..."
                    ));
                    // A replacement that spawns but never reports ready
                    // must not outlive the error return: it is not in
                    // `children`, so the caller's teardown would miss it.
                    let mut replacement = ManagedChild::spawn(&stage.spec)?;
                    if let Err(error) = await_ready(&mut replacement, &stage.readiness).await {
                        replacement.terminate_group();
                        return Err(error);
                    }
                    print_line("flight-controller ready");
                    children[index] = replacement;
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{start_stages, supervise};
    use crate::backend::Stage;
    use crate::process::{ManagedChild, ProcessSpec};
    use crate::readiness::Readiness;

    /// Builds a stage running `script` under sh, with `marker` planted in
    /// argv so liveness is observable from outside via pgrep.
    fn stage(name: &'static str, script: &str, marker: &str, readiness: Readiness) -> Stage {
        let log = std::env::temp_dir().join(format!("plt_xtask_{marker}.log"));
        Stage {
            spec: ProcessSpec {
                name,
                program: "sh".to_owned(),
                args: vec!["-c".to_owned(), script.to_owned(), marker.to_owned()],
                cwd: None,
                env: Vec::new(),
                remove_env: Vec::new(),
                log_path: log,
            },
            readiness,
        }
    }

    fn marker_alive(marker: &str) -> bool {
        std::process::Command::new("pgrep")
            .args(["-f", marker])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn wait_until(deadline: Duration, mut done: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + deadline;
        while Instant::now() < end {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        done()
    }

    /// A stage that never reports ready must not leave the stages started
    /// before it (or itself) running.
    #[tokio::test]
    async fn readiness_failure_tears_down_every_started_stage() {
        let a = format!("plt_xt_rdy_a_{}", std::process::id());
        let b = format!("plt_xt_rdy_b_{}", std::process::id());
        let stages = vec![
            stage(
                "first",
                "echo READY; sleep 30",
                &a,
                Readiness::LogContains {
                    needle: "READY",
                    timeout_s: 5,
                },
            ),
            stage(
                "second",
                "sleep 30",
                &b,
                Readiness::LogContains {
                    needle: "NEVER_APPEARS",
                    timeout_s: 1,
                },
            ),
        ];

        let outcome = start_stages(&stages).await;

        assert!(outcome.is_err(), "the second stage can never become ready");
        assert!(
            wait_until(Duration::from_secs(5), || !marker_alive(&a)
                && !marker_alive(&b)),
            "both stages must be torn down after the readiness failure"
        );
    }

    /// A flight-controller replacement that spawns but never reports
    /// ready is not in `children`, so the supervisor must kill it before
    /// returning the error.
    #[tokio::test]
    async fn failed_restart_kills_the_unready_replacement() {
        let marker = format!("plt_xt_fcr_{}", std::process::id());
        let fc = stage(
            "flight-controller",
            "sleep 30",
            &marker,
            Readiness::LogContains {
                needle: "NEVER_APPEARS",
                timeout_s: 1,
            },
        );
        // The supervised child exits immediately, triggering the restart.
        let dying = stage(
            "flight-controller",
            "exit 7",
            "plt_xt_dying",
            Readiness::LogContains {
                needle: "",
                timeout_s: 1,
            },
        );
        let child = ManagedChild::spawn(&dying.spec).expect("dying stage spawns");
        let mut children = vec![child];
        let stages = vec![fc];

        let outcome = supervise(&mut children, &stages).await;

        assert!(outcome.is_err(), "the replacement can never become ready");
        assert!(
            wait_until(Duration::from_secs(5), || !marker_alive(&marker)),
            "the unready replacement must not outlive the error"
        );
    }
}
