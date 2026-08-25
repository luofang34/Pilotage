#![allow(clippy::expect_used, clippy::panic)]

use std::io::Read as _;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use super::{OwnerReaper, ReapableOwner, spawn_with_reaper};
use crate::AviateSupervisorError;

const REAPER_FIXTURE_ENV: &str = "PILOTAGE_OWNER_REAPER_FIXTURE";

#[test]
fn blocking_child_fixture() {
    if std::env::var_os(REAPER_FIXTURE_ENV).is_none() {
        return;
    }
    let mut input = std::io::stdin();
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).expect("read fixture input");
}

#[test]
fn reaper_failure_prevents_child_spawn() {
    let child_spawned = std::sync::atomic::AtomicBool::new(false);

    let result = spawn_with_reaper(
        || Err(AviateSupervisorError::protocol("injected reaper failure")),
        || {
            child_spawned.store(true, std::sync::atomic::Ordering::SeqCst);
            Err(AviateSupervisorError::protocol("unexpected child spawn"))
        },
    );
    let Err(error) = result else {
        panic!("the launch must stop before child spawn");
    };

    assert!(matches!(error, AviateSupervisorError::Protocol { .. }));
    assert!(!child_spawned.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn repeated_owner_drop_reaps_each_child() {
    for _ in 0..3 {
        let (child, input) = spawn_blocking_child();
        let pid = child.id();
        let (completion, completed) = mpsc::channel();
        let reaper = OwnerReaper::spawn_worker(Some(completion)).expect("spawn named reaper");
        let owner = ReapableOwner {
            child: Some(child),
            reaper: Some(reaper),
        };

        drop(input);
        drop(owner);
        completed
            .recv_timeout(Duration::from_secs(5))
            .expect("observe exact child reap");

        assert!(
            crate::inspection::inspect_lifetime(pid)
                .expect("inspect reaped child")
                .is_none(),
            "the named reaper removes the exact child lifetime"
        );
    }
}

#[test]
fn owner_drop_hands_off_a_live_child_without_blocking() {
    let (child, input) = spawn_blocking_child();
    let pid = child.id();
    let (reaped_sender, reaped) = mpsc::channel();
    let reaper = OwnerReaper::spawn_worker(Some(reaped_sender)).expect("spawn named reaper");
    let owner = ReapableOwner {
        child: Some(child),
        reaper: Some(reaper),
    };
    let (drop_sender, dropped) = mpsc::channel();
    let dropper = std::thread::spawn(move || {
        drop(owner);
        drop_sender.send(()).expect("report owner drop");
    });

    dropped
        .recv_timeout(Duration::from_secs(5))
        .expect("owner drop returns while the child is live");
    assert!(
        crate::inspection::inspect_lifetime(pid)
            .expect("inspect handed-off child")
            .is_some(),
        "the reaper owns the live child"
    );
    drop(input);
    reaped
        .recv_timeout(Duration::from_secs(5))
        .expect("observe handed-off child reap");
    assert!(
        crate::inspection::inspect_lifetime(pid)
            .expect("inspect reaped handed-off child")
            .is_none(),
        "the reaper removes the handed-off child lifetime"
    );
    dropper.join().expect("join owner dropper");
}

fn spawn_blocking_child() -> (Child, ChildStdin) {
    let mut child = Command::new(std::env::current_exe().expect("find test executable"))
        .args([
            "--exact",
            "process::reaper::tests::blocking_child_fixture",
            "--nocapture",
        ])
        .env(REAPER_FIXTURE_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn blocking fixture");
    let input = child.stdin.take().expect("take fixture input");
    (child, input)
}
