//! Managed child processes: spawn with captured logs and their own
//! process group, health checks, and group teardown.
//!
//! Each stage runs in its OWN process group so a terminal ctrl-c (which
//! signals the launcher's group) never kills stages out from under the
//! ordered teardown, and so teardown can signal a stage's whole subtree
//! (gz spawns helpers) without unsafe code — the group is signalled via
//! the `kill` utility rather than `libc::killpg`.

use std::fs::File;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::XtaskError;

/// Grace period between TERM and KILL during teardown.
const TERM_GRACE: Duration = Duration::from_secs(2);

/// One process a session stage runs.
#[derive(Debug)]
pub struct ProcessSpec {
    /// Stage name for progress and errors.
    pub name: &'static str,
    /// Program to execute.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Working directory, when it matters.
    pub cwd: Option<PathBuf>,
    /// Environment entries set on top of the inherited environment.
    pub env: Vec<(String, String)>,
    /// Environment names removed (e.g. `DISPLAY` for headless gz).
    pub remove_env: Vec<&'static str>,
    /// File capturing the process's stdout+stderr.
    pub log_path: PathBuf,
}

/// A spawned stage.
#[derive(Debug)]
pub struct ManagedChild {
    /// Stage name.
    pub name: &'static str,
    /// Captured-output path.
    pub log_path: PathBuf,
    child: Child,
}

impl ManagedChild {
    /// Spawns `spec` with stdout/stderr captured to its log file.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::Spawn`] (or [`XtaskError::Io`] for the log
    /// file) when the process cannot start.
    pub fn spawn(spec: &ProcessSpec) -> Result<Self, XtaskError> {
        let log = File::create(&spec.log_path).map_err(|source| XtaskError::Io {
            context: "creating a stage log file",
            source,
        })?;
        let log_err = log.try_clone().map_err(|source| XtaskError::Io {
            context: "cloning a stage log handle",
            source,
        })?;
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        for key in &spec.remove_env {
            command.env_remove(key);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn().map_err(|source| XtaskError::Spawn {
            name: spec.name,
            source,
        })?;
        Ok(Self {
            name: spec.name,
            log_path: spec.log_path.clone(),
            child,
        })
    }

    /// `Ok(())` while running; the stage's exit status once it stopped.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::StageDied`] with the log tail when the
    /// process has exited.
    pub fn check_running(&mut self) -> Result<(), XtaskError> {
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => Err(XtaskError::StageDied {
                name: self.name,
                status: status.to_string(),
                tail: self.log_tail(12),
            }),
            Err(source) => Err(XtaskError::Io {
                context: "polling a stage",
                source,
            }),
        }
    }

    /// The last `lines` of the stage's captured log.
    pub fn log_tail(&self, lines: usize) -> String {
        let Ok(content) = std::fs::read_to_string(&self.log_path) else {
            return String::from("<log unreadable>");
        };
        let all: Vec<&str> = content.lines().collect();
        let start = all.len().saturating_sub(lines);
        all[start..].join("\n")
    }

    /// Terminates the stage's whole process group: TERM, a bounded
    /// grace wait, then an unconditional group KILL, then reap. The
    /// leader exiting only ends the grace wait early — descendants
    /// that ignored TERM outlive it in the same group, so the KILL is
    /// sent regardless (killing an already-empty group is harmless).
    pub fn terminate_group(&mut self) {
        let pid = self.child.id();
        signal_group(pid, "-TERM");
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        signal_group(pid, "-KILL");
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// Signals a process group via the `kill` utility (no unsafe `killpg`).
fn signal_group(pid: u32, signal: &str) {
    Command::new("kill")
        .arg(signal)
        .arg("--")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

/// Whether any member of `pid`'s process group is still alive (signal 0
/// probes without delivering).
#[cfg(test)]
pub(crate) fn group_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg("--")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{ManagedChild, ProcessSpec, group_alive};

    fn spec(name: &'static str, script: &str, log: &std::path::Path) -> ProcessSpec {
        ProcessSpec {
            name,
            program: "sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
            cwd: None,
            env: Vec::new(),
            remove_env: Vec::new(),
            log_path: log.to_path_buf(),
        }
    }

    fn wait_until(deadline: Duration, mut done: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + deadline;
        while Instant::now() < end {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        done()
    }

    /// The leader exiting must not spare its descendants: a grandchild
    /// that ignores TERM still dies from the unconditional group KILL.
    #[test]
    fn group_kill_reaches_term_ignoring_descendants_after_the_leader_exits() {
        let log = std::env::temp_dir().join(format!("plt_xtask_grp_{}.log", std::process::id()));
        let mut child = ManagedChild::spawn(&spec(
            "group-test",
            // The grandchild ignores TERM and holds the group; the
            // leader exits immediately.
            "sh -c 'trap \"\" TERM; sleep 30' & exit 0",
            &log,
        ))
        .expect("test stage spawns");
        let pid = child.child.id();

        assert!(
            wait_until(Duration::from_secs(5), || child.check_running().is_err()),
            "the leader must exit on its own"
        );
        assert!(
            group_alive(pid),
            "the TERM-ignoring grandchild must be holding the group"
        );

        child.terminate_group();

        assert!(
            wait_until(Duration::from_secs(5), || !group_alive(pid)),
            "the group KILL must reach the grandchild even though the leader already exited"
        );
        std::fs::remove_file(&log).ok();
    }
}
