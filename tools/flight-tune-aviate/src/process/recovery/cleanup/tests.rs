use std::os::unix::process::CommandExt as _;

use crate::document::ProcessStartIdentity;
use crate::{AviateSupervisorError, inspection};

use super::{
    WaitControl, recover_processes_same_boot_blocking, require_live_group_anchor, validate_group,
};

#[test]
fn reused_process_group_with_another_start_token_is_not_signaled() {
    let arguments = vec!["/bin/sleep".to_owned(), "60".to_owned()];
    let argument_digest = inspection::digest_arguments(&arguments);
    let mut command = std::process::Command::new(&arguments[0]);
    command.arg(&arguments[1]).process_group(0);
    let mut child = ReapedChild::spawn(command);
    let actual = inspection::inspect_process(child.id(), argument_digest)
        .expect("inspect isolated group member")
        .expect("isolated group member is live");
    let snapshot =
        inspection::process_group_snapshot(child.id()).expect("inspect isolated process group");
    let mut stale = actual;
    change_start_token(&mut stale.start);

    validate_group(&stale, &snapshot).expect("the reused group has the expected containment");
    let result = require_live_group_anchor(&stale, None, &snapshot);
    let still_running = child.is_running();
    child.stop_and_wait();

    assert!(matches!(
        result,
        Err(AviateSupervisorError::RecoveryBlocked { .. })
    ));
    assert!(still_running, "recovery does not signal the reused group");
}

#[test]
fn owner_and_group_waits_share_one_deadline() {
    let arguments = vec!["/bin/sleep".to_owned(), "60".to_owned()];
    let argument_digest = inspection::digest_arguments(&arguments);
    let mut owner = spawn_group_member(&arguments);
    let mut gate = spawn_group_member(&arguments);
    let owner_identity = inspect_group_member(&owner, argument_digest, "owner");
    let gate_identity = inspect_group_member(&gate, argument_digest, "gate");
    let mut wait = OwnerExitWait::new(&mut owner);

    let result = recover_processes_same_boot_blocking(
        &owner_identity,
        &gate_identity,
        None,
        std::time::Duration::from_millis(1),
        &mut wait,
    );

    // Prints the result, because recovery inspects a third time inside this
    // call and a mismatch originating there would otherwise be discarded —
    // pid and arguments built, then thrown away by a bare `matches!`.
    assert!(
        matches!(
            result,
            Err(AviateSupervisorError::Timeout {
                operation: "wait for recovered process group removal",
            })
        ),
        "unexpected recovery result: {result:?}"
    );
    assert_eq!(
        wait.park_calls, 1,
        "the group wait cannot restart the deadline"
    );
    assert!(
        gate.is_running(),
        "recovery does not stop the live gate group"
    );
    gate.stop_and_wait();
}

struct OwnerExitWait<'a> {
    now: std::time::Instant,
    owner: &'a mut ReapedChild,
    park_calls: u32,
}

impl<'a> OwnerExitWait<'a> {
    fn new(owner: &'a mut ReapedChild) -> Self {
        Self {
            now: std::time::Instant::now(),
            owner,
            park_calls: 0,
        }
    }
}

impl WaitControl for OwnerExitWait<'_> {
    fn now(&mut self) -> std::time::Instant {
        self.now
    }

    fn park_for_poll_blocking(&mut self, duration: std::time::Duration) {
        self.park_calls = self.park_calls.wrapping_add(1);
        self.now = self.now.checked_add(duration).expect("advance fake clock");
        if self.park_calls == 1 {
            self.owner.stop_and_wait();
        }
    }
}

fn spawn_group_member(arguments: &[String]) -> ReapedChild {
    let mut command = std::process::Command::new(&arguments[0]);
    command.arg(&arguments[1]).process_group(0);
    ReapedChild::spawn(command)
}

/// Inspects one member, saying WHICH member when it cannot.
///
/// The two are spawned and inspected in a fixed order, and an inspection that
/// fails names a suspect by where it sat in that order: the owner is inspected
/// after a whole second spawn has run, the gate immediately after its own. A
/// failure that only ever names the gate says something a failure naming
/// either one does not.
fn inspect_group_member(
    child: &ReapedChild,
    argument_digest: flight_tune::Digest,
    role: &str,
) -> crate::document::ProcessIdentity {
    inspection::inspect_process(child.id(), argument_digest)
        .unwrap_or_else(|error| panic!("inspect the {role}: {error}"))
        .unwrap_or_else(|| panic!("the {role} is live"))
}

fn change_start_token(start: &mut ProcessStartIdentity) {
    match start {
        ProcessStartIdentity::Linux { start_ticks, .. } => {
            *start_ticks = start_ticks.wrapping_add(1);
        }
        ProcessStartIdentity::MacOs { start_abstime, .. } => {
            *start_abstime = start_abstime.wrapping_add(1);
        }
    }
}

struct ReapedChild {
    child: std::process::Child,
    reaped: bool,
}

impl ReapedChild {
    fn spawn(mut command: std::process::Command) -> Self {
        Self {
            child: command.spawn().expect("spawn isolated group member"),
            reaped: false,
        }
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn is_running(&mut self) -> bool {
        self.child
            .try_wait()
            .expect("inspect isolated group member")
            .is_none()
    }

    fn stop_and_wait(&mut self) {
        self.child.kill().expect("stop isolated group member");
        self.child.wait().expect("reap isolated group member");
        self.reaped = true;
    }
}

impl Drop for ReapedChild {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        match self.child.kill() {
            Ok(()) | Err(_) => {}
        }
        match self.child.wait() {
            Ok(_) | Err(_) => {}
        }
    }
}
