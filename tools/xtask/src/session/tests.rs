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
    claim_supervisor, resolve_manifest, start_stages, supervise, verify_listening_port,
    viewer_stage,
};
use crate::backend::{SessionContext, Stage};
use crate::cli::Profile;
use crate::error::XtaskError;
use crate::process::{ManagedChild, ProcessSpec};
use crate::readiness::{Readiness, await_ready};

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn runtime_manifest_wins_over_deleted_compiled_checkout() {
    let runtime = Path::new("/active/Pilotage/tools/xtask");
    let deleted_compiled = Path::new("/deleted/worktree/tools/xtask");

    assert_eq!(resolve_manifest(Some(runtime), deleted_compiled), runtime);
}

#[test]
fn compiled_manifest_is_the_outside_cargo_fallback() {
    let compiled = Path::new("/compiled/Pilotage/tools/xtask");

    assert_eq!(resolve_manifest(None, compiled), compiled);
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

/// A stage that never reports ready must not leave the stages started
/// before it (or itself) running.
#[tokio::test]
async fn readiness_failure_tears_down_every_started_stage() {
    let fifo_a = make_fifo("rdyf_a");
    let fifo_b = make_fifo("rdyf_b");
    let watch_a = watch_fifo(fifo_a.clone());
    let watch_b = watch_fifo(fifo_b.clone());
    let stages = vec![
        fifo_stage("first", &fifo_a, ready()),
        fifo_stage("second", &fifo_b, never(1)),
    ];
    let (_keep, mut cancel) = tokio::sync::watch::channel(false);

    let outcome = start_stages(&stages, &mut cancel).await;

    assert!(outcome.is_err(), "the second stage can never become ready");
    expect_event(&watch_a, "open", "first stage starts");
    expect_event(&watch_b, "open", "second stage starts");
    expect_event(&watch_a, "eof", "first stage group dies");
    expect_event(&watch_b, "eof", "second stage group dies");
}

/// A flight-controller replacement that spawns but never reports
/// ready is not in `children`, so the supervisor must kill it before
/// returning the error.
#[tokio::test]
async fn failed_restart_kills_the_unready_replacement() {
    let fifo = make_fifo("fcr");
    let watch = watch_fifo(fifo.clone());
    let fc = fifo_stage("flight-controller", &fifo, never(1));
    let dying = ProcessSpec {
        name: "flight-controller",
        program: "sh".to_owned(),
        args: vec!["-c".to_owned(), "exit 7".to_owned()],
        cwd: None,
        env: Vec::new(),
        remove_env: Vec::new(),
        log_path: fifo.with_extension("dying.log"),
    };
    let child = ManagedChild::spawn(&dying).expect("dying stage spawns");
    let mut children = vec![child];
    let stages = vec![fc];
    let (_keep, mut cancel) = tokio::sync::watch::channel(false);

    let outcome = supervise(&mut children, &stages, &mut cancel).await;

    assert!(outcome.is_err(), "the replacement can never become ready");
    expect_event(&watch, "open", "replacement starts");
    expect_event(&watch, "eof", "unready replacement dies with the error");
}

/// Ctrl-c while a stage's readiness is still pending must tear down
/// the not-yet-recorded child and every stage started before it.
#[tokio::test]
async fn cancellation_during_startup_tears_down_everything() {
    let fifo_a = make_fifo("cans_a");
    let fifo_b = make_fifo("cans_b");
    let watch_a = watch_fifo(fifo_a.clone());
    let watch_b = watch_fifo(fifo_b.clone());
    let stages = vec![
        fifo_stage("first", &fifo_a, ready()),
        // A long deadline: only the cancellation can end this wait.
        fifo_stage("second", &fifo_b, never(30)),
    ];
    let (cancel_tx, mut cancel) = tokio::sync::watch::channel(false);
    // The ctrl-c arrives once the second stage is provably running
    // and its readiness wait is in progress.
    let trigger = std::thread::spawn(move || {
        let event = watch_b
            .recv_timeout(EVENT_TIMEOUT)
            .expect("second stage starts");
        assert_eq!(event, "open");
        cancel_tx.send(true).ok();
        let event = watch_b
            .recv_timeout(EVENT_TIMEOUT)
            .expect("second stage group dies");
        assert_eq!(event, "eof");
    });

    let outcome = start_stages(&stages, &mut cancel).await;

    assert!(
        matches!(outcome, Err(XtaskError::Cancelled)),
        "cancellation is reported as the typed requested-stop"
    );
    expect_event(&watch_a, "open", "first stage starts");
    expect_event(&watch_a, "eof", "first stage group dies");
    trigger
        .join()
        .expect("second stage started, cancel fired, its group died");
}

/// Ctrl-c during a flight-controller restart must stop promptly and
/// kill the not-yet-ready replacement.
#[tokio::test]
async fn cancellation_during_restart_kills_the_replacement() {
    let fifo = make_fifo("canr");
    let watch = watch_fifo(fifo.clone());
    let fc = fifo_stage("flight-controller", &fifo, never(30));
    let dying = ProcessSpec {
        name: "flight-controller",
        program: "sh".to_owned(),
        args: vec!["-c".to_owned(), "exit 7".to_owned()],
        cwd: None,
        env: Vec::new(),
        remove_env: Vec::new(),
        log_path: fifo.with_extension("dying.log"),
    };
    let child = ManagedChild::spawn(&dying).expect("dying stage spawns");
    let mut children = vec![child];
    let stages = vec![fc];
    let (cancel_tx, mut cancel) = tokio::sync::watch::channel(false);
    let trigger = std::thread::spawn(move || {
        let event = watch
            .recv_timeout(EVENT_TIMEOUT)
            .expect("replacement starts");
        assert_eq!(event, "open");
        cancel_tx.send(true).ok();
        let event = watch
            .recv_timeout(EVENT_TIMEOUT)
            .expect("replacement group dies");
        assert_eq!(event, "eof");
    });

    let outcome = supervise(&mut children, &stages, &mut cancel).await;

    assert!(outcome.is_ok(), "a requested stop during restart is clean");
    trigger.join().expect("replacement started, then died");
}

/// A pid-marker write failure must not leave the session running:
/// nothing can coordinate with a supervisor that has no marker.
#[test]
fn marker_write_failure_tears_down_the_session() {
    let fifo = make_fifo("marker");
    let watch = watch_fifo(fifo.clone());
    let stage = fifo_stage("holder", &fifo, ready());
    let child = ManagedChild::spawn(&stage.spec).expect("holder spawns");
    expect_event(&watch, "open", "holder starts");
    let mut children = vec![child];
    let unwritable = std::env::temp_dir()
        .join(format!("plt_xt_absent_{}", std::process::id()))
        .join("supervisor.pid");

    let outcome = claim_supervisor(&unwritable, &mut children);

    assert!(outcome.is_err(), "the marker path cannot be written");
    assert!(children.is_empty(), "teardown drains every child");
    expect_event(&watch, "eof", "holder group dies");
}

/// Artifacts alone no longer short-circuit the preflight: without a
/// recorded content stamp the build reruns (fail closed toward rebuilding),
/// and only a stamp matching the current sources + toolchain skips it. The
/// temp root has no `scripts/`, so an attempted build fails loudly — `Ok`
/// proves nothing ran.
#[test]
fn web_assets_preflight_skips_only_with_a_matching_content_stamp() {
    let root = std::env::temp_dir().join(format!("plt_xt_web_ok_{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("temp root is created");
    // A real (empty) git repository, so the content stamp is computable.
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()
            .expect("git init runs")
            .success(),
        "git init succeeds"
    );
    for rel in WEB_RUNTIME_ARTIFACTS {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("artifact has a parent"))
            .expect("artifact dir is created");
        std::fs::write(&path, b"generated").expect("artifact is written");
    }

    // Present artifacts WITHOUT a stamp rebuild — and the missing build
    // script surfaces that as a typed failure.
    let unstamped = prepare_web_assets(&root);
    assert!(
        matches!(unstamped, Err(XtaskError::CommandFailed { .. })),
        "artifacts without a stamp must rebuild, got {unstamped:?}"
    );

    // Recording the CURRENT stamp makes the same checkout skip the build.
    let current = crate::session::preflight::stamp::source_stamp(
        &root,
        &crate::session::preflight::WEB_RUNTIME_SOURCES,
        &[&["rustc", "--version"], &["wasm-bindgen", "--version"]],
    )
    .expect("stamp computes inside a git repository");
    crate::session::preflight::stamp::write_stamp(
        &root.join("target/xtask-stamps/web-runtime.stamp"),
        &current,
    );
    let outcome = prepare_web_assets(&root);
    std::fs::remove_dir_all(&root).ok();

    assert!(
        outcome.is_ok(),
        "a matching content stamp skips the build, got {outcome:?}"
    );
}

/// A checkout missing the generated web runtime triggers a build. With no
/// `scripts/build-web-instruments.sh` under the temp root the build cannot
/// run, and the failure is surfaced as a typed fault — never silently served
/// as a dead viewer.
#[test]
fn web_assets_preflight_builds_and_surfaces_failure_when_absent() {
    let root = std::env::temp_dir().join(format!("plt_xt_web_missing_{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("temp root is created");

    let outcome = prepare_web_assets(&root);
    std::fs::remove_dir_all(&root).ok();

    assert!(
        matches!(outcome, Err(XtaskError::CommandFailed { .. })),
        "a missing web runtime must trigger a build whose failure is typed, got {outcome:?}"
    );
}

#[test]
fn listening_port_must_match_the_requested_port() {
    assert!(verify_listening_port(4433, 4433).is_ok());
    assert!(matches!(
        verify_listening_port(4433, 4434),
        Err(XtaskError::PortMismatch {
            requested: 4433,
            actual: 4434,
        })
    ));
}

/// Reserves an ephemeral port by binding then releasing it, so the
/// spawned server can claim it. The reserve→claim window is tiny and
/// loopback-local; a collision only reruns the test.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("an ephemeral port binds")
        .local_addr()
        .expect("the bound address is readable")
        .port()
}

/// Minimal HTTP/1.0 GET returning the raw response. HTTP/1.0 makes the
/// server close the connection at end-of-body, so the read completes on
/// EOF without content-length parsing.
fn http_get(port: u16, path: &str) -> String {
    use std::io::Write;
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))
        .expect("the viewer server accepts the request connection");
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .expect("the request is written");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("the response is read to EOF");
    response
}

/// Every viewer response must carry `Cache-Control: no-store` so the
/// browser can never present a cached client that has diverged from the
/// working-tree source. Spawns the real `viewer_stage` process on an
/// ephemeral port and reads the header off the wire.
#[tokio::test]
async fn viewer_server_sets_cache_control_no_store() {
    let repo_root = std::env::temp_dir().join(format!("plt_xt_viewer_{}", std::process::id()));
    let web = repo_root.join("clients/web");
    std::fs::create_dir_all(&web).expect("temp clients/web is created");
    std::fs::write(web.join("index.html"), "<title>viewer</title>")
        .expect("the viewer entrypoint is written");

    let ctx = SessionContext {
        repo_root: repo_root.clone(),
        host_port: 0,
        viewer_port: free_port(),
        profile: Profile::Simulation,
        log_dir: repo_root.clone(),
    };
    let stage = viewer_stage(&ctx).expect("the viewer stage plans");
    let mut child = ManagedChild::spawn(&stage.spec).expect("the viewer server spawns");
    let ready = await_ready(&mut child, &stage.readiness).await;

    // Read the header off the wire only once the server accepts, but
    // always tear the process group down before asserting.
    let response = ready.is_ok().then(|| http_get(ctx.viewer_port, "/"));
    child.terminate_group();
    std::fs::remove_dir_all(&repo_root).ok();

    ready.expect("the viewer server accepts connections");
    let headers = response
        .unwrap_or_default()
        .split("\r\n\r\n")
        .next()
        .unwrap_or_default()
        .to_owned();
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("cache-control: no-store"),
        "every viewer response must carry Cache-Control: no-store; got headers:\n{headers}"
    );
}
