//! What becomes of a running session's children: readiness, restart,
//! cancellation, and the teardown each of them owes.

#![allow(clippy::expect_used, clippy::panic)]

use super::*;

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

    let outcome = supervise(
        &mut children,
        &stages,
        &NoRestartWork,
        &test_context(),
        &mut cancel,
    )
    .await;

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

    let outcome = supervise(
        &mut children,
        &stages,
        &NoRestartWork,
        &test_context(),
        &mut cancel,
    )
    .await;

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

#[tokio::test]
async fn cancellation_while_re_establishing_a_restart_stops_the_session() {
    // The work a restart puts back can WAIT: re-issuing a handshake waits for
    // a simulator the operator may have just closed, which is the ordinary way
    // a session ends. Performed on the runtime thread it stops the runtime
    // polling, and the task watching for ctrl-c stops with it — so the
    // operator presses it, nothing happens at all, and the reasonable next
    // move is `kill -9`, which orphans the host and the viewer.
    //
    // Without the race this test does not fail, it HANGS.
    let fifo = make_fifo("canw");
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let backend = WaitingRestartWork {
        entered: entered_tx,
        release: std::sync::Mutex::new(Some(release_rx)),
    };
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
    let stages = vec![fifo_stage("flight-controller", &fifo, never(30))];
    let (cancel_tx, mut cancel) = tokio::sync::watch::channel(false);
    let trigger = std::thread::spawn(move || {
        entered_rx
            .recv_timeout(EVENT_TIMEOUT)
            .expect("the restart work is reached");
        cancel_tx.send(true).ok();
    });

    // Bounded, because without the race this DEADLOCKS rather than failing:
    // the work waits for a sender the test cannot drop while it is blocked
    // here. An unbounded hang burns a CI job's whole timeout and reports
    // "cancelled" with no message; this reports the assertion.
    let outcome = tokio::time::timeout(
        EVENT_TIMEOUT,
        supervise(
            &mut children,
            &stages,
            &backend,
            &test_context(),
            &mut cancel,
        ),
    )
    .await
    .expect("a stop requested while the restart work waited was never honoured");

    assert!(
        outcome.is_ok(),
        "a stop requested while the restart work waited did not end cleanly"
    );
    trigger.join().expect("the restart work was reached");
    drop(release_tx);
}
