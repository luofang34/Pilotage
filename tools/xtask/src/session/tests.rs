//! Session lifecycle tests: every failure and cancellation path
//! must leave zero surviving process groups, proven with fifo
//! open/EOF process events rather than polling.

#![allow(clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use super::preflight::{WEB_RUNTIME_ARTIFACTS, prepare_web_assets};
use super::{
    claim_supervisor, resolve_manifest, start_stages, verify_listening_port, viewer_stage,
};

mod lifecycle;
mod plumbing;

/// A backend that plans nothing and restores nothing.
///
/// `supervise` only needs one to ask whether a restarted stage has anything to
/// put back; these tests are about the restart loop itself, so the honest
/// stand-in is a backend for which the answer is no.
struct NoRestartWork;

impl crate::backend::SimBackend for NoRestartWork {
    fn name(&self) -> &'static str {
        "test"
    }
    fn host_adapter(&self) -> &'static str {
        "reference"
    }
    fn host_env(&self, _ctx: &SessionContext) -> Vec<(String, String)> {
        Vec::new()
    }
    fn plan(&self, _ctx: &SessionContext) -> Result<Vec<Stage>, crate::error::XtaskError> {
        Ok(Vec::new())
    }
    fn stale_process_patterns(&self) -> Vec<&'static str> {
        Vec::new()
    }
    fn reset(&self, _repo_root: &Path) -> Result<(), crate::error::XtaskError> {
        Ok(())
    }
}

fn test_context() -> SessionContext {
    SessionContext {
        repo_root: std::env::temp_dir(),
        host_port: 0,
        viewer_port: 0,
        profile: crate::cli::Profile::Simulation,
        log_dir: std::env::temp_dir(),
        lan: false,
        web_root: std::env::temp_dir(),
        instance: 0,
        ready_timeout_s: crate::backend::DEFAULT_READY_TIMEOUT_S,
        sim_camera: false,
    }
}
use super::supervise::supervise;
use crate::backend::{SessionContext, Stage};
use crate::cli::Profile;
use crate::error::XtaskError;
use crate::process::{ManagedChild, ProcessSpec};
use crate::readiness::Readiness;

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn runtime_manifest_wins_over_deleted_compiled_checkout() {
    let runtime = Path::new("/active/Pilotage/tools/xtask");
    let deleted_compiled = Path::new("/deleted/worktree/tools/xtask");

    assert_eq!(resolve_manifest(Some(runtime), deleted_compiled), runtime);
}

/// A stage that opens `fifo` for writing, prints READY, and parks.
/// The fifo is the synchronization primitive: its open is the
/// stage-started event, and EOF fires only when every process
/// holding the write end — the whole group — is gone.
fn fifo_stage(name: &'static str, fifo: &std::path::Path, readiness: Readiness) -> Stage {
    Stage {
        spec: ProcessSpec {
            name,
            program: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                format!("exec 9>{}; echo READY; sleep 30", fifo.display()),
            ],
            cwd: None,
            env: Vec::new(),
            remove_env: Vec::new(),
            log_path: fifo.with_extension("log"),
        },
        readiness,
    }
}

fn ready() -> Readiness {
    Readiness::LogContains {
        needle: "READY",
        timeout_s: 10,
    }
}

fn never(timeout_s: u64) -> Readiness {
    Readiness::LogContains {
        needle: "NEVER_APPEARS",
        timeout_s,
    }
}

fn make_fifo(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("plt_xt_{tag}_{}.fifo", std::process::id()));
    std::fs::remove_file(&path).ok();
    let status = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo creates the fifo");
    path
}

/// Watches `fifo` from its own thread: sends "open" once the stage
/// opens the write end and "eof" once every write end is closed.
fn watch_fifo(fifo: std::path::PathBuf) -> mpsc::Receiver<&'static str> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut file = std::fs::File::open(&fifo).expect("fifo opens once its stage starts");
        tx.send("open").ok();
        let mut sink = Vec::new();
        file.read_to_end(&mut sink).ok();
        tx.send("eof").ok();
        std::fs::remove_file(&fifo).ok();
    });
    rx
}

fn expect_event(rx: &mpsc::Receiver<&'static str>, expected: &str, what: &str) {
    let event = rx
        .recv_timeout(EVENT_TIMEOUT)
        .unwrap_or_else(|_| panic!("timed out: {what}"));
    assert_eq!(event, expected, "{what}");
}

/// A backend whose restart work WAITS, the way re-issuing a handshake waits
/// for a simulator to answer.
struct WaitingRestartWork {
    entered: std::sync::mpsc::SyncSender<()>,
    release: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl crate::backend::SimBackend for WaitingRestartWork {
    fn name(&self) -> &'static str {
        "test"
    }
    fn host_adapter(&self) -> &'static str {
        "reference"
    }
    fn host_env(&self, _ctx: &SessionContext) -> Vec<(String, String)> {
        Vec::new()
    }
    fn plan(&self, _ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        Ok(Vec::new())
    }
    fn stale_process_patterns(&self) -> Vec<&'static str> {
        Vec::new()
    }
    fn reset(&self, _repo_root: &Path) -> Result<(), XtaskError> {
        Ok(())
    }
    fn before_stage_restart(
        &self,
        _ctx: &SessionContext,
        stage_name: &str,
    ) -> Option<Box<dyn FnOnce() -> Result<(), XtaskError> + Send>> {
        if stage_name != "flight-controller" {
            return None;
        }
        let entered = self.entered.clone();
        let release = self.release.lock().ok()?.take()?;
        Some(Box::new(move || {
            entered.send(()).ok();
            // Returns when the test drops its sender, so nothing here outlives
            // the test or depends on a duration.
            release.recv().ok();
            Ok(())
        }))
    }
}

/// Instance 0 keeps the run directory the reset scripts read the
/// supervisor marker from; another instance never shares it.
#[test]
fn each_instance_runs_in_a_directory_of_its_own() {
    assert_eq!(
        super::sim_run_dir(0),
        std::path::PathBuf::from("target/xtask-sim")
    );
    assert_eq!(
        super::sim_run_dir(5),
        std::path::PathBuf::from("target/xtask-sim/instance-5")
    );
    assert_ne!(super::sim_run_dir(5), super::sim_run_dir(6));
}

/// A session claims its own host and viewer ports before anything of
/// its backend's.
#[test]
fn a_session_claims_its_host_and_viewer_ports() {
    use crate::conflicts::{Claim, session_claims};

    let mut ctx = test_context();
    ctx.host_port = 4533;
    ctx.viewer_port = 8180;
    let claims = session_claims(&ctx, &NoRestartWork);
    assert!(matches!(
        claims.first(),
        Some(Claim::Udp { port: 4533, .. })
    ));
    assert!(matches!(claims.get(1), Some(Claim::Tcp { port: 8180, .. })));
}
