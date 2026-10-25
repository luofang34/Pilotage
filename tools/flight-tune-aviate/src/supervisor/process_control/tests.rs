use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

use super::stop_process_group;

fn group_leader() -> Child {
    Command::new("/bin/sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn group leader")
}

fn group_of(child: &Child) -> u32 {
    child.id()
}

/// Blocks until `child` exits and leaves it unreaped, so its group holds only a zombie.
fn wait_until_zombie(child: &Child) {
    let raw = i32::try_from(child.id()).expect("child PID fits a POSIX PID");
    let pid = Pid::from_raw(raw).expect("child PID is not zero");
    waitid(
        WaitId::Pid(pid),
        WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
    )
    .expect("wait for child exit");
}

#[test]
fn a_group_holding_only_an_unreaped_zombie_counts_as_stopped() {
    let mut leader = group_leader();
    leader.kill().expect("kill group leader");
    wait_until_zombie(&leader);

    let stopped = stop_process_group(group_of(&leader));

    leader.wait().expect("reap group leader");
    assert!(
        stopped.is_ok(),
        "a zombie-only group must count as stopped: {stopped:?}"
    );
}

#[test]
fn a_group_with_a_running_member_receives_the_stop_signal() {
    let mut leader = group_leader();

    stop_process_group(group_of(&leader)).expect("stop a running group");

    let status = leader.wait().expect("reap group leader");
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&status),
        Some(rustix::process::Signal::KILL.as_raw()),
        "the running member stops on the group signal"
    );
}

#[test]
fn a_reaped_group_counts_as_stopped() {
    let mut leader = group_leader();
    leader.kill().expect("kill group leader");
    leader.wait().expect("reap group leader");

    stop_process_group(group_of(&leader)).expect("a reaped group has nothing left to stop");
}
