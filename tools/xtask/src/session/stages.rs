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
/// stage; until then python3 serves the session's web root exactly like
/// the recorded runbook.
pub(crate) fn viewer_stage(ctx: &SessionContext) -> Result<Stage, XtaskError> {
    let entry = ctx.repo_root.join(VIEWER_DIR).join("index.html");
    if !entry.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "viewer entrypoint",
            path: entry,
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
