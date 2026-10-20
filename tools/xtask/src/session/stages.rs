//! The stages every session runs after its backend's own: the session
//! host and the static viewer, plus the directories they work in.

use std::path::{Path, PathBuf};

use crate::backend::{SessionContext, SimBackend, Stage};
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The viewer source tree, relative to the repository root.
const VIEWER_DIR: &str = "clients/web";
/// The connect manifest each viewer serves.
pub(crate) const MANIFEST_NAME: &str = "session.json";

/// The host stage, with the environment DERIVED from the backend rather
/// than handed in.
///
/// Taking a ready-made environment is what let a caller assemble the
/// right one and then pass a different one — two correct halves and a
/// wrong join, which no test of either half can see. With no such
/// parameter there is nothing to get wrong at the call site.
pub(crate) fn host_stage(
    ctx: &SessionContext,
    backend: &dyn SimBackend,
) -> Result<Stage, XtaskError> {
    let adapter = backend.host_adapter();
    let mut env = crate::backend::host_env_for(backend, ctx);
    if ctx.sim_camera {
        env.extend(crate::camera::host_env(crate::camera::port(ctx)?));
    }
    env.push((
        "PILOTAGE_AVIATE_PROFILE".to_owned(),
        ctx.profile.as_env_value().to_owned(),
    ));
    if std::env::var_os("RUST_LOG").is_none() {
        env.push(("RUST_LOG".to_owned(), "info".to_owned()));
    }
    Ok(Stage {
        spec: ProcessSpec {
            name: "session-host",
            program: ctx
                .repo_root
                .join("target/release/pilotage-session-host")
                .display()
                .to_string(),
            args: host_arguments(ctx, adapter)?,
            cwd: Some(ctx.repo_root.clone()),
            env,
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "session-host"),
        },
        readiness: Readiness::HostListening { timeout_s: 60 },
    })
}

fn host_arguments(ctx: &SessionContext, adapter: &str) -> Result<Vec<String>, XtaskError> {
    if ctx.lan && ctx.join_token_file.is_none() {
        return Err(XtaskError::Usage {
            message: "--lan requires --join-token-file with an absolute local path".to_owned(),
        });
    }
    let mut args = vec![
        "--port".to_owned(),
        ctx.host_port.to_string(),
        "--adapter".to_owned(),
        adapter.to_owned(),
        "--bind".to_owned(),
        if ctx.lan { "0.0.0.0" } else { "127.0.0.1" }.to_owned(),
    ];
    if let Some(path) = &ctx.join_token_file {
        if !path.is_absolute() {
            return Err(XtaskError::Usage {
                message: "--join-token-file requires an absolute local path".to_owned(),
            });
        }
        args.extend(["--join-token-file".to_owned(), path.display().to_string()]);
    }
    Ok(args)
}

/// The static viewer: this xtask binary's `serve-viewer` subcommand
/// serving the session's web root with caching disabled, so the browser
/// always runs the working-tree client.
pub(crate) fn viewer_stage(ctx: &SessionContext, xtask: &Path) -> Result<Stage, XtaskError> {
    let entry = ctx.repo_root.join(VIEWER_DIR).join("index.html");
    if !entry.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "viewer entrypoint",
            path: entry,
            hint: "run from the Pilotage repository root",
        });
    }
    // Loopback by default: serving the working tree to the network is
    // an explicit request, never a side effect of running a sim.
    let bind = if ctx.lan { "0.0.0.0" } else { "127.0.0.1" };
    Ok(Stage {
        spec: ProcessSpec {
            name: "viewer",
            program: xtask.display().to_string(),
            args: vec![
                "serve-viewer".to_owned(),
                "--dir".to_owned(),
                ctx.web_root.display().to_string(),
                "--port".to_owned(),
                ctx.viewer_port.to_string(),
                "--bind".to_owned(),
                bind.to_owned(),
            ],
            cwd: Some(ctx.web_root.clone()),
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

/// The viewer source tree, which is also the web root of the session
/// that owns `clients/web/session.json`.
pub(crate) fn viewer_source(repo_root: &Path) -> PathBuf {
    repo_root.join(VIEWER_DIR)
}

/// Builds a web root of the session's own under `run_dir`: every entry of
/// the viewer tree linked in, except the connect manifest.
///
/// A viewer tab whose pinned certificate has gone stale re-reads
/// `/session.json` and connects to whatever it names. Served from the
/// shared tree, every viewer would hand out one session's manifest, and
/// a tab for one vehicle would silently attach to another. The links keep
/// the served client identical to the working tree.
///
/// # Errors
///
/// Returns [`XtaskError::Io`] when the directory or a link cannot be made.
pub(crate) fn private_web_root(repo_root: &Path, run_dir: &Path) -> Result<PathBuf, XtaskError> {
    let io = |context: &'static str| move |source| XtaskError::Io { context, source };
    let source = viewer_source(repo_root);
    let root = run_dir.join("web");
    match std::fs::remove_dir_all(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io("clearing the session's web root")(error)),
    }
    std::fs::create_dir_all(&root).map_err(io("creating the session's web root"))?;
    let entries = std::fs::read_dir(&source).map_err(io("listing the viewer tree"))?;
    for entry in entries {
        let entry = entry.map_err(io("listing the viewer tree"))?;
        let name = entry.file_name();
        if name == MANIFEST_NAME {
            continue;
        }
        std::os::unix::fs::symlink(entry.path(), root.join(&name))
            .map_err(io("linking the viewer tree into the session's web root"))?;
    }
    Ok(root)
}

#[cfg(test)]
mod tests;
