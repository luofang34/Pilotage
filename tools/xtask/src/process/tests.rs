//! Process-group teardown proven with fifo open/EOF process events.

#![allow(clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

use super::{ManagedChild, ProcessSpec};

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

/// The leader exiting must not spare its descendants: a grandchild
/// that ignores TERM still dies from the unconditional group KILL.
/// The fifo synchronizes on process events, not polling: its open is
/// the grandchild-started event, and EOF fires only when every
/// process holding the write end — the whole group — is gone.
#[test]
fn group_kill_reaches_term_ignoring_descendants_after_the_leader_exits() {
    let tag = format!("plt_grp_{}", std::process::id());
    let fifo = std::env::temp_dir().join(format!("{tag}.fifo"));
    let log = std::env::temp_dir().join(format!("{tag}.log"));
    std::fs::remove_file(&fifo).ok();
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo creates the fifo");

    let (tx, rx) = mpsc::channel();
    let reader_path = fifo.clone();
    std::thread::spawn(move || {
        let mut file =
            std::fs::File::open(&reader_path).expect("fifo opens once the grandchild starts");
        tx.send("open").ok();
        let mut sink = Vec::new();
        file.read_to_end(&mut sink).ok();
        tx.send("eof").ok();
        std::fs::remove_file(&reader_path).ok();
    });

    // The grandchild ignores TERM and holds the fifo; the leader
    // exits immediately.
    let mut child = ManagedChild::spawn(&ProcessSpec {
        name: "group-test",
        program: "sh".to_owned(),
        args: vec![
            "-c".to_owned(),
            format!(
                "sh -c 'trap \"\" TERM; exec 9>{}; sleep 30' & exit 0",
                fifo.display()
            ),
        ],
        cwd: None,
        env: Vec::new(),
        remove_env: Vec::new(),
        log_path: log.clone(),
    })
    .expect("test stage spawns");

    let event = rx.recv_timeout(EVENT_TIMEOUT).expect("grandchild starts");
    assert_eq!(event, "open", "the grandchild is holding the group");
    // The leader exits on its own; reaped or not, the group must die.
    child.child.wait().ok();

    child.terminate_group();

    let event = rx
        .recv_timeout(EVENT_TIMEOUT)
        .expect("the group KILL reaches the grandchild");
    assert_eq!(event, "eof", "every group member is gone");
    std::fs::remove_file(&log).ok();
}

/// A panic between spawn and teardown (a failed request in a test, a
/// bug in the launcher) must not orphan the stage: dropping the handle
/// during the unwind stops the whole group. The fifo's EOF fires only
/// once every process holding its write end is gone.
#[test]
fn a_panic_before_teardown_still_stops_the_stage_group() {
    let tag = format!("plt_drop_{}", std::process::id());
    let fifo = std::env::temp_dir().join(format!("{tag}.fifo"));
    let log = std::env::temp_dir().join(format!("{tag}.log"));
    std::fs::remove_file(&fifo).ok();
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo creates the fifo");

    let (tx, rx) = mpsc::channel();
    let reader_path = fifo.clone();
    std::thread::spawn(move || {
        let mut file = std::fs::File::open(&reader_path).expect("fifo opens once the stage starts");
        tx.send("open").ok();
        let mut sink = Vec::new();
        file.read_to_end(&mut sink).ok();
        tx.send("eof").ok();
        std::fs::remove_file(&reader_path).ok();
    });

    let spec = ProcessSpec {
        name: "drop-test",
        program: "sh".to_owned(),
        args: vec![
            "-c".to_owned(),
            format!("exec 9>{}; sleep 30 & sleep 30", fifo.display()),
        ],
        cwd: None,
        env: Vec::new(),
        remove_env: Vec::new(),
        log_path: log.clone(),
    };
    let unwound = std::panic::catch_unwind(|| {
        let _child = ManagedChild::spawn(&spec).expect("test stage spawns");
        let event = rx.recv_timeout(EVENT_TIMEOUT).expect("the stage starts");
        assert_eq!(event, "open", "the stage is holding the fifo");
        panic!("the request against the stage failed");
    });

    assert!(unwound.is_err(), "the closure panicked");
    let event = rx
        .recv_timeout(EVENT_TIMEOUT)
        .expect("dropping the handle stops the stage group");
    assert_eq!(event, "eof", "every group member is gone");
    std::fs::remove_file(&log).ok();
}
