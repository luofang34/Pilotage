#![allow(clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;

use super::{
    ExecutableObservation, ProcessSource, STABLE_SNAPSHOT_ATTEMPTS, inspect_process_from,
    observe_open_executable, parse_stat,
};
use crate::AviateSupervisorError;
use crate::document::ProcessStartIdentity;
use crate::inspection::LifetimeIdentity;

#[test]
fn stat_parser_uses_the_last_command_boundary() {
    let mut fields = vec!["S", "42", "43", "44"];
    fields.extend(std::iter::repeat_n("0", 15));
    fields.push("99");
    let stat = format!("77 (name) with ) marks) {}", fields.join(" "));

    let parsed = parse_stat(&stat).expect("valid stat");

    assert_eq!(parsed, ("S", 42, 43, 44, 99));
}

#[test]
fn inspection_discards_an_exec_surface_change() {
    let lifetime = lifetime();
    let target_command = command("/target");
    let old_executable = executable("/old", 1);
    let new_executable = executable("/new", 2);
    let mut source = ScriptedSource::new(
        vec![
            Some(lifetime.clone()),
            Some(lifetime.clone()),
            Some(lifetime.clone()),
            Some(lifetime),
        ],
        vec![target_command.clone(); 4],
        vec![
            old_executable,
            new_executable.clone(),
            new_executable.clone(),
            new_executable.clone(),
        ],
    );
    let digest = crate::inspection::digest_argument_bytes(
        target_command
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty()),
    );

    let identity = inspect_process_from(&mut source, 77, digest)
        .expect("inspect stable process")
        .expect("stable process is present");

    assert_eq!(identity.executable, new_executable.path);
    assert_eq!(identity.executable_digest, new_executable.digest);
    assert!(source.is_empty(), "inspection consumes both attempts");
}

#[test]
fn inspection_rejects_bounded_process_image_churn() {
    let lifetime = lifetime();
    let mut lifetimes = Vec::new();
    let mut commands = Vec::new();
    let mut executables = Vec::new();
    for _ in 0..STABLE_SNAPSHOT_ATTEMPTS {
        lifetimes.extend([Some(lifetime.clone()), Some(lifetime.clone())]);
        commands.extend([command("/old"), command("/new")]);
        executables.extend([executable("/old", 1), executable("/new", 2)]);
    }
    let mut source = ScriptedSource::new(lifetimes, commands, executables);
    let digest = crate::inspection::digest_argument_bytes([b"/new".as_slice()]);

    let error = inspect_process_from(&mut source, 77, digest)
        .expect_err("reject an unstable process image");

    assert!(matches!(
        error,
        AviateSupervisorError::IdentityMismatch { .. }
    ));
    assert!(
        source.is_empty(),
        "inspection uses the bounded attempt count"
    );
}

#[test]
fn inspection_rejects_pid_reuse_after_a_missing_image_surface() {
    let mut replacement = lifetime();
    replacement.start = ProcessStartIdentity::Linux {
        boot_id: "11111111-1111-1111-1111-111111111111".to_owned(),
        start_ticks: 100,
    };
    let mut source = ScriptedSource {
        lifetimes: [Some(lifetime()), Some(replacement)].into(),
        commands: [None].into(),
        executables: VecDeque::new(),
    };
    let digest = crate::inspection::digest_argument_bytes([b"/new".as_slice()]);

    let error = inspect_process_from(&mut source, 77, digest)
        .expect_err("reject a replacement process lifetime");

    assert!(matches!(
        error,
        AviateSupervisorError::IdentityMismatch { .. }
    ));
    assert!(source.is_empty(), "inspection stops at the reused lifetime");
}

#[test]
fn inspection_rejects_pid_reuse_between_snapshot_attempts() {
    let first = lifetime();
    let mut replacement = lifetime();
    replacement.start = ProcessStartIdentity::Linux {
        boot_id: "11111111-1111-1111-1111-111111111111".to_owned(),
        start_ticks: 100,
    };
    let target_command = command("/target");
    let mut source = ScriptedSource::new(
        vec![
            Some(first.clone()),
            Some(first),
            Some(replacement.clone()),
            Some(replacement),
        ],
        vec![target_command.clone(); 4],
        vec![
            executable("/old", 1),
            executable("/new", 2),
            executable("/new", 2),
            executable("/new", 2),
        ],
    );
    let digest = crate::inspection::digest_argument_bytes(
        target_command
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty()),
    );

    let error = inspect_process_from(&mut source, 77, digest)
        .expect_err("reject process reuse between attempts");

    assert!(matches!(
        error,
        AviateSupervisorError::IdentityMismatch { .. }
    ));
    assert!(source.is_empty(), "inspection stops at the reused lifetime");
}

#[test]
fn inspection_retries_a_missing_surface_for_the_same_lifetime() {
    let target_command = command("/target");
    let target_executable = executable("/target", 3);
    let mut source = ScriptedSource {
        lifetimes: [
            Some(lifetime()),
            Some(lifetime()),
            Some(lifetime()),
            Some(lifetime()),
        ]
        .into(),
        commands: [
            None,
            Some(target_command.clone()),
            Some(target_command.clone()),
        ]
        .into(),
        executables: [
            Some(target_executable.clone()),
            Some(target_executable.clone()),
        ]
        .into(),
    };
    let digest = crate::inspection::digest_argument_bytes(
        target_command
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty()),
    );

    let identity = inspect_process_from(&mut source, 77, digest)
        .expect("retry same-lifetime missing surface")
        .expect("same process remains present");

    assert_eq!(identity.executable, target_executable.path);
    assert!(source.is_empty(), "inspection consumes the retry surface");
}

#[test]
fn inspection_accepts_confirmed_absence_after_a_missing_surface() {
    let mut source = ScriptedSource {
        lifetimes: [Some(lifetime()), None].into(),
        commands: [None].into(),
        executables: VecDeque::new(),
    };

    let identity = inspect_process_from(
        &mut source,
        77,
        crate::inspection::digest_argument_bytes([b"/target".as_slice()]),
    )
    .expect("inspect absent process");

    assert!(identity.is_none(), "confirmed absence has no identity");
    assert!(source.is_empty(), "absence stops the inspection");
}

#[test]
fn held_executable_keeps_its_path_and_digest_when_a_link_changes() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("create executable observation root");
    let first = root.path().join("first");
    let second = root.path().join("second");
    let active = root.path().join("active");
    std::fs::write(&first, b"first executable").expect("write first executable");
    std::fs::write(&second, b"second executable").expect("write second executable");
    symlink(&first, &active).expect("link first executable");
    let held = std::fs::File::open(&active).expect("hold first executable");
    std::fs::remove_file(&active).expect("remove first executable link");
    symlink(&second, &active).expect("link second executable");

    let observation =
        observe_open_executable(held, &active).expect("observe held executable identity");

    assert_eq!(observation.path, first);
    assert_eq!(
        observation.digest,
        crate::inspection::digest_file(&first).expect("digest first executable")
    );
}

struct ScriptedSource {
    lifetimes: VecDeque<Option<LifetimeIdentity>>,
    commands: VecDeque<Option<Vec<u8>>>,
    executables: VecDeque<Option<ExecutableObservation>>,
}

impl ScriptedSource {
    fn new(
        lifetimes: Vec<Option<LifetimeIdentity>>,
        commands: Vec<Vec<u8>>,
        executables: Vec<ExecutableObservation>,
    ) -> Self {
        Self {
            lifetimes: lifetimes.into(),
            commands: commands.into_iter().map(Some).collect(),
            executables: executables.into_iter().map(Some).collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.lifetimes.is_empty() && self.commands.is_empty() && self.executables.is_empty()
    }
}

impl ProcessSource for ScriptedSource {
    fn check_deadline(&mut self) -> Result<(), AviateSupervisorError> {
        Ok(())
    }

    fn lifetime(&mut self, _pid: u32) -> Result<Option<LifetimeIdentity>, AviateSupervisorError> {
        Ok(self.lifetimes.pop_front().expect("scripted lifetime"))
    }

    fn command(&mut self, _pid: u32) -> Result<Option<Vec<u8>>, AviateSupervisorError> {
        Ok(self.commands.pop_front().expect("scripted command"))
    }

    fn executable(
        &mut self,
        _pid: u32,
    ) -> Result<Option<ExecutableObservation>, AviateSupervisorError> {
        Ok(self.executables.pop_front().expect("scripted executable"))
    }
}

#[test]
fn inspection_outwaits_a_command_line_in_flight() {
    // A freshly-spawned process reports an EMPTY command line until its
    // execve completes; the kernel is saying "in flight", not "someone
    // else". The inspection must wait it out, not call it a mismatch.
    let lifetime_value = lifetime();
    let target_command = command("/target");
    let target_executable = executable("/target", 3);
    let mut source = ScriptedSource::new(
        vec![
            Some(lifetime_value.clone()),
            Some(lifetime_value.clone()),
            Some(lifetime_value.clone()),
            Some(lifetime_value),
        ],
        vec![
            Vec::new(),
            Vec::new(),
            target_command.clone(),
            target_command.clone(),
        ],
        vec![
            target_executable.clone(),
            target_executable.clone(),
            target_executable.clone(),
            target_executable,
        ],
    );
    let digest = crate::inspection::digest_argument_bytes(
        target_command
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty()),
    );

    let identity = inspect_process_from(&mut source, 77, digest)
        .expect("outwait the exec window")
        .expect("the process is present");

    assert_eq!(identity.observed_argv_digest, Some(digest));
    assert!(source.is_empty(), "inspection consumes both snapshots");
}

#[test]
fn a_command_line_that_never_arrives_refuses_as_unstabilized() {
    // A permanently empty command line (a zombie, a kernel thread) is
    // indeterminate, and an indeterminate identity must refuse as one —
    // never as "the arguments differ", which accuses a live process of
    // being somebody else.
    let mut lifetimes = Vec::new();
    let mut commands = Vec::new();
    let mut executables = Vec::new();
    for _ in 0..STABLE_SNAPSHOT_ATTEMPTS {
        lifetimes.extend([Some(lifetime()), Some(lifetime())]);
        commands.extend([Vec::new(), Vec::new()]);
        executables.extend([executable("/target", 3), executable("/target", 3)]);
    }
    let mut source = ScriptedSource::new(lifetimes, commands, executables);
    let digest = crate::inspection::digest_argument_bytes([b"/target".as_slice()]);

    let error = inspect_process_from(&mut source, 77, digest)
        .expect_err("refuse a command line that never arrives");

    let text = error.to_string();
    assert!(
        text.contains("stabilize") && !text.contains("differ"),
        "{text}"
    );
}

fn lifetime() -> LifetimeIdentity {
    LifetimeIdentity {
        pid: 77,
        process_group: 77,
        session_id: 77,
        parent_pid: 42,
        real_user_id: 501,
        start: ProcessStartIdentity::Linux {
            boot_id: "11111111-1111-1111-1111-111111111111".to_owned(),
            start_ticks: 99,
        },
        is_zombie: false,
    }
}

fn command(executable: &str) -> Vec<u8> {
    let mut command = executable.as_bytes().to_vec();
    command.push(0);
    command
}

fn executable(path: &str, byte: u8) -> ExecutableObservation {
    ExecutableObservation {
        path: path.into(),
        digest: flight_tune::Digest::from_bytes([byte; 32]),
        device: 1,
        inode: u64::from(byte),
        bytes: 1024,
    }
}
