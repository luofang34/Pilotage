//! Session lifecycle tests: every failure and cancellation path
//! must leave zero surviving process groups, proven with fifo
//! open/EOF process events rather than polling.

#![allow(clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

use super::{claim_supervisor, start_stages, supervise, verify_listening_port};
use crate::backend::Stage;
use crate::error::XtaskError;
use crate::process::{ManagedChild, ProcessSpec};
use crate::readiness::Readiness;

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

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
