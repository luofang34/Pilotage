use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ChildStdin;
use std::time::Duration;

use crate::AviateSupervisorError;
use crate::document::{ProcessIdentity, TargetProcessContract};

mod reaper;
mod recovery;
mod startup;

pub use recovery::{
    RECOVERY_REQUEST_SCHEMA_VERSION, RecoveryOutcome, RecoveryRequest,
    recover_supervised_process_blocking,
};

/// Schema version for one external supervision attestation.
pub const SUPERVISION_ATTESTATION_SCHEMA_VERSION: u16 = 1;

/// One exact process-supervision launch request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisedProcessRequest {
    /// Process-supervisor helper executable.
    pub supervisor_executable: PathBuf,
    /// Required SHA-256 digest of the helper executable.
    pub supervisor_executable_digest: flight_tune::Digest,
    /// Target executable.
    pub target_executable: PathBuf,
    /// Required SHA-256 digest of the target executable.
    pub target_executable_digest: flight_tune::Digest,
    /// Complete target argument vector after argument zero.
    pub target_arguments: Vec<String>,
    /// Complete target environment. The launcher clears the inherited environment.
    pub target_environment: BTreeMap<String, String>,
    /// Required process-tree behavior for the authorized target executable.
    pub target_process_contract: TargetProcessContract,
    /// Exact target current directory.
    pub target_current_directory: PathBuf,
    /// New durable supervisor-document root.
    pub storage_root: PathBuf,
    /// Existing empty mode-0700 runtime root.
    pub runtime_root: PathBuf,
    /// Absent private executable-artifact root.
    pub artifact_root: PathBuf,
    /// Digest of the exact run intent that authorizes this launch.
    pub run_intent_digest: flight_tune::Digest,
    /// Maximum duration for target authorization.
    pub startup_timeout: Duration,
    /// Maximum duration for exact process-group cleanup.
    pub cleanup_timeout: Duration,
}

/// Exact durable digests that authorize recovery of one supervised run.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisionAttestation {
    /// Attestation schema version.
    pub schema_version: u16,
    /// Digest of the exact run intent.
    pub run_intent_digest: flight_tune::Digest,
    /// Digest of the exact spawn intent.
    pub spawn_intent_digest: flight_tune::Digest,
    /// Digest of the exact owner and launch-gate identity document.
    pub process_identity_digest: flight_tune::Digest,
    /// Exact process owner identity.
    pub supervisor_identity: ProcessIdentity,
    /// Exact launch-gate identity.
    pub target_gate_identity: ProcessIdentity,
    /// Recovery request that binds the external campaign journal to this run.
    pub recovery_request: RecoveryRequest,
}

/// One armed launch that cannot start its target before explicit release.
#[must_use = "persist the attestation before release or drop the prepared launch"]
pub struct PreparedAviateProcess {
    launch: Option<startup::PreparedLaunch>,
    attestation: SupervisionAttestation,
}

impl PreparedAviateProcess {
    /// Arm one exact owner and launch gate without starting the target.
    pub fn prepare_blocking(
        request: SupervisedProcessRequest,
    ) -> Result<Self, AviateSupervisorError> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let launch = startup::prepare_supported(request)?;
            let attestation = startup::prepared_attestation(&launch).clone();
            Ok(Self {
                launch: Some(launch),
                attestation,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = request;
            Err(AviateSupervisorError::UnsupportedPlatform)
        }
    }

    /// Get the pre-release evidence for the external campaign journal.
    #[must_use]
    pub const fn supervision_attestation(&self) -> &SupervisionAttestation {
        &self.attestation
    }

    /// Release the target after the caller durably stores the attestation.
    pub fn release_blocking(mut self) -> Result<ManagedAviateProcess, AviateSupervisorError> {
        let launch = self.launch.take().ok_or_else(|| {
            AviateSupervisorError::protocol("the prepared launch was already consumed")
        })?;
        startup::release_supported(launch)
    }

    /// Close the launch gate and wait for durable cleanup evidence.
    pub fn cancel_blocking(mut self) -> Result<RecoveryOutcome, AviateSupervisorError> {
        let launch = self.launch.take().ok_or_else(|| {
            AviateSupervisorError::protocol("the prepared launch was already consumed")
        })?;
        startup::cancel_supported(launch)
    }
}

impl Drop for PreparedAviateProcess {
    fn drop(&mut self) {
        if let Some(launch) = self.launch.as_mut() {
            startup::cancel_prepared(launch);
        }
    }
}

/// One running target whose owner holds its durable writer lease.
#[must_use = "dropping the handle requests cleanup but does not wait for evidence"]
pub struct ManagedAviateProcess {
    supervisor: reaper::ReapableOwner,
    parent_lifetime: Option<ChildStdin>,
    supervisor_identity: ProcessIdentity,
    target_identity: ProcessIdentity,
    attestation: SupervisionAttestation,
    recovery: RecoveryRequest,
    cleanup_timeout: Duration,
    terminated: bool,
}

impl ManagedAviateProcess {
    /// Verify that the exact supervisor and target lifetimes are still live.
    pub fn ensure_running_blocking(&self) -> Result<(), AviateSupervisorError> {
        startup::verify_live(&self.supervisor_identity, "supervisor")?;
        startup::verify_live(&self.target_identity, "target")
    }

    /// Request cleanup and wait for its durable terminal receipt.
    pub fn terminate_blocking(&mut self) -> Result<RecoveryOutcome, AviateSupervisorError> {
        self.parent_lifetime.take();
        let supervisor = self.supervisor.child_mut()?;
        startup::wait_for_supervisor_terminal(
            supervisor,
            &self.supervisor_identity,
            self.cleanup_timeout,
        )?;
        let outcome = recover_supervised_process_blocking(&self.recovery)?;
        self.terminated = true;
        Ok(outcome)
    }

    /// Get the exact attested target identity.
    #[must_use]
    pub const fn target_identity(&self) -> &ProcessIdentity {
        &self.target_identity
    }

    /// Get the exact durable supervision attestation.
    #[must_use]
    pub const fn supervision_attestation(&self) -> &SupervisionAttestation {
        &self.attestation
    }
}

impl Drop for ManagedAviateProcess {
    fn drop(&mut self) {
        if !self.terminated {
            self.parent_lifetime.take();
            tracing::warn!(
                supervisor_pid = self.supervisor_identity.pid,
                "Aviate process handle dropped; cleanup continues in the owner"
            );
        }
    }
}
