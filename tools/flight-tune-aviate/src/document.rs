use std::path::PathBuf;

use flight_tune::Digest;
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA_VERSION: u16 = 1;
pub(crate) const SPAWN_INTENT_NAME: &str = "supervisor-spawn-intent.json";
pub(crate) const PROCESS_IDENTITY_NAME: &str = "supervisor-process-identity.json";
pub(crate) const TARGET_ATTESTATION_NAME: &str = "supervisor-target-attestation.json";
pub(crate) const TERMINAL_RECEIPT_NAME: &str = "supervisor-terminal-receipt.json";
pub(crate) const RECOVERY_RECEIPT_NAME: &str = "supervisor-recovery-receipt.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchoredExecutable {
    pub(crate) path: PathBuf,
    pub(crate) digest: Digest,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchoredDirectory {
    pub(crate) path: PathBuf,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) mode: u32,
}

/// A high-resolution operating-system process start identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "platform", rename_all = "snake_case")]
pub enum ProcessStartIdentity {
    /// Linux boot identity and `/proc/<pid>/stat` clock tick.
    Linux {
        /// Kernel boot identity.
        boot_id: String,
        /// Process start clock tick since boot.
        start_ticks: u64,
    },
    /// Darwin process start time from `proc_pidinfo` and `proc_pid_rusage`.
    MacOs {
        /// Darwin boot-session UUID.
        boot_session_uuid: String,
        /// Whole seconds since the Unix epoch.
        seconds: u64,
        /// Microseconds within the start second.
        microseconds: u32,
        /// Mach absolute time at process start.
        start_abstime: u64,
    },
}

/// One operating-system boot identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "platform", rename_all = "snake_case")]
pub(crate) enum BootIdentity {
    /// Linux kernel boot identity.
    Linux {
        /// Kernel boot identity.
        boot_id: String,
    },
    /// Darwin boot-session identity.
    MacOs {
        /// Darwin boot-session UUID.
        boot_session_uuid: String,
    },
}

impl ProcessStartIdentity {
    pub(crate) fn boot_identity(&self) -> BootIdentity {
        match self {
            Self::Linux { boot_id, .. } => BootIdentity::Linux {
                boot_id: boot_id.clone(),
            },
            Self::MacOs {
                boot_session_uuid, ..
            } => BootIdentity::MacOs {
                boot_session_uuid: boot_session_uuid.clone(),
            },
        }
    }
}

impl BootIdentity {
    pub(crate) fn is_valid(&self) -> bool {
        let value = match self {
            Self::Linux { boot_id } => boot_id,
            Self::MacOs { boot_session_uuid } => boot_session_uuid,
        };
        value.len() == 36
            && value == &value.to_ascii_lowercase()
            && value.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
            && value.bytes().any(|byte| byte != b'0' && byte != b'-')
    }
}

/// One exact operating-system process identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    /// Process identifier.
    pub pid: u32,
    /// Process-group identifier.
    pub process_group: u32,
    /// Session identifier.
    pub session_id: u32,
    /// Parent process identifier observed by the operating system.
    pub parent_pid: u32,
    /// Real user identifier observed by the operating system.
    pub real_user_id: u32,
    /// High-resolution process lifetime identity.
    pub start: ProcessStartIdentity,
    /// Canonical executable path observed for the process.
    pub executable: PathBuf,
    /// SHA-256 digest of the observed executable bytes.
    pub executable_digest: Digest,
    /// SHA-256 digest of the canonical argument vector bound at launch.
    pub launch_argv_digest: Digest,
    /// SHA-256 digest of the operating-system argument vector, when available.
    pub observed_argv_digest: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpawnIntent {
    pub(crate) schema_version: u16,
    pub(crate) run_intent_digest: Digest,
    pub(crate) correlation_nonce: Digest,
    pub(crate) release_secret_digest: Digest,
    pub(crate) supervisor_executable: AnchoredExecutable,
    pub(crate) target_executable: AnchoredExecutable,
    pub(crate) target_arguments: Vec<String>,
    pub(crate) target_argv_digest: Digest,
    pub(crate) target_environment_digest: Digest,
    pub(crate) target_current_directory: AnchoredDirectory,
    pub(crate) target_stdio: TargetStdio,
    pub(crate) target_process_contract: TargetProcessContract,
    pub(crate) cleanup_timeout_millis: u64,
    pub(crate) runtime_root: AnchoredDirectory,
    pub(crate) artifact_root: AnchoredDirectory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetStdio {
    Null,
}

/// Process-tree behavior required from one supervised target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetProcessContract {
    /// The target and all descendants stay in the launch process group until exit.
    ///
    /// The target must not use `setsid`, change its process group, or start a
    /// detached descendant. The caller must bind this contract to a reviewed
    /// executable digest before it releases the target.
    RetainProcessGroup,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessIdentityDocument {
    pub(crate) schema_version: u16,
    pub(crate) run_intent_digest: Digest,
    pub(crate) spawn_intent_digest: Digest,
    pub(crate) correlation_nonce: Digest,
    pub(crate) supervisor: ProcessIdentity,
    pub(crate) target_gate: ProcessIdentity,
}

/// The exact target identity observed after the launch gate opened.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetAttestation {
    /// Process document schema version.
    pub schema_version: u16,
    /// Exact run intent that authorized this target.
    pub run_intent_digest: Digest,
    /// Exact target identity after `exec`.
    pub target: ProcessIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalReceipt {
    pub(crate) schema_version: u16,
    pub(crate) run_intent_digest: Digest,
    pub(crate) spawn_intent_digest: Digest,
    pub(crate) process_identity_digest: Digest,
    pub(crate) target_attestation_digest: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryReceipt {
    pub(crate) schema_version: u16,
    pub(crate) run_intent_digest: Digest,
    pub(crate) spawn_intent_digest: Digest,
    pub(crate) process_identity_digest: Digest,
    pub(crate) target_attestation_digest: Option<Digest>,
    pub(crate) prior_boot_identity: BootIdentity,
    pub(crate) recovery_boot_identity: BootIdentity,
}
