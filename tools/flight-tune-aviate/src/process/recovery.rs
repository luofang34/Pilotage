use std::path::PathBuf;
use std::time::Duration;

use crate::AviateSupervisorError;
use crate::document::{
    BootIdentity, ProcessIdentityDocument, RecoveryReceipt, SCHEMA_VERSION, SpawnIntent,
    TERMINAL_RECEIPT_NAME, TargetAttestation, TerminalReceipt,
};
use crate::lease_store::LeaseStore;

mod cleanup;
mod documents;

/// Schema version for one external recovery request.
pub const RECOVERY_REQUEST_SCHEMA_VERSION: u16 = 1;

/// One exact recovery request for a supervised process family.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequest {
    /// Recovery-request schema version.
    pub schema_version: u16,
    /// Existing durable supervisor-document root.
    pub storage_root: PathBuf,
    /// Required run-intent digest.
    pub run_intent_digest: flight_tune::Digest,
    /// Required process-supervisor executable digest.
    pub supervisor_executable_digest: flight_tune::Digest,
    /// Required target executable digest.
    pub target_executable_digest: flight_tune::Digest,
    /// Exact spawn-intent digest from launch authorization.
    pub expected_spawn_intent_digest: flight_tune::Digest,
    /// Exact process-identity digest from launch authorization.
    pub expected_process_identity_digest: flight_tune::Digest,
    /// Maximum duration for one recovery cleanup operation.
    pub cleanup_timeout_millis: u64,
}

/// Durable evidence that permits the caller to archive one stopped run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "recovery evidence must be retained by the caller"]
pub enum RecoveryOutcome {
    /// Exact same-boot or owner cleanup has a durable receipt.
    Terminal {
        /// Digest of the terminal receipt.
        receipt_digest: flight_tune::Digest,
    },
    /// A different operating-system boot has a durable cleanup receipt.
    BootChange {
        /// Digest of the boot-change recovery receipt.
        receipt_digest: flight_tune::Digest,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TargetEvidence {
    pub(super) attestation: TargetAttestation,
    pub(super) digest: flight_tune::Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RecoveryState {
    pub(super) intent: SpawnIntent,
    pub(super) intent_digest: flight_tune::Digest,
    pub(super) processes: ProcessIdentityDocument,
    pub(super) process_digest: flight_tune::Digest,
    pub(super) target: Option<TargetEvidence>,
}

/// Recover one exact supervised run under its durable writer lease.
pub fn recover_supervised_process_blocking(
    request: &RecoveryRequest,
) -> Result<RecoveryOutcome, AviateSupervisorError> {
    validate_request(request)?;
    let store = LeaseStore::open_existing(&request.storage_root)?;
    let loaded = documents::load(&store, request)?;
    let current_boot = crate::inspection::current_boot_identity()?;
    let prior_boot = loaded.state.processes.supervisor.start.boot_identity();
    if !prior_boot.is_valid() || !current_boot.is_valid() {
        return Err(AviateSupervisorError::invalid_document(
            "boot identity",
            "the recovery boot identity is invalid",
        ));
    }
    if let Some(outcome) = documents::existing_outcome(request, &loaded, &current_boot)? {
        if prior_boot == current_boot {
            cleanup::recover_same_boot(
                &loaded.state,
                Duration::from_millis(request.cleanup_timeout_millis),
            )?;
        }
        cleanup::verify_resources_clean(&loaded.state)?;
        return Ok(outcome);
    }
    if prior_boot == current_boot {
        cleanup::recover_same_boot(
            &loaded.state,
            Duration::from_millis(request.cleanup_timeout_millis),
        )?;
        cleanup::cleanup_resources(&loaded.state)?;
        publish_terminal(&store, request, &loaded.state)
    } else {
        cleanup::cleanup_resources(&loaded.state)?;
        publish_boot_change(&store, request, &loaded.state, prior_boot, current_boot)
    }
}

fn validate_request(request: &RecoveryRequest) -> Result<(), AviateSupervisorError> {
    if request.schema_version != RECOVERY_REQUEST_SCHEMA_VERSION
        || request.run_intent_digest.is_zero()
        || request.supervisor_executable_digest.is_zero()
        || request.target_executable_digest.is_zero()
        || request.expected_spawn_intent_digest.is_zero()
        || request.expected_process_identity_digest.is_zero()
        || request.cleanup_timeout_millis == 0
    {
        return Err(AviateSupervisorError::invalid_request(
            "the recovery request is incomplete",
        ));
    }
    Ok(())
}

fn publish_terminal(
    store: &LeaseStore,
    request: &RecoveryRequest,
    state: &RecoveryState,
) -> Result<RecoveryOutcome, AviateSupervisorError> {
    let receipt = TerminalReceipt {
        schema_version: SCHEMA_VERSION,
        run_intent_digest: request.run_intent_digest,
        spawn_intent_digest: state.intent_digest,
        process_identity_digest: state.process_digest,
        target_attestation_digest: state.target.as_ref().map(|target| target.digest),
    };
    let digest = store.publish(TERMINAL_RECEIPT_NAME, &receipt)?;
    verify_terminal_readback(store, &receipt, digest)?;
    Ok(RecoveryOutcome::Terminal {
        receipt_digest: digest,
    })
}

fn verify_terminal_readback(
    store: &LeaseStore,
    expected: &TerminalReceipt,
    expected_digest: flight_tune::Digest,
) -> Result<(), AviateSupervisorError> {
    let (actual, actual_digest): (TerminalReceipt, _) = store.read(TERMINAL_RECEIPT_NAME)?;
    if actual != *expected || actual_digest != expected_digest {
        return Err(AviateSupervisorError::invalid_document(
            "terminal receipt",
            "the terminal receipt readback changed",
        ));
    }
    Ok(())
}

fn publish_boot_change(
    store: &LeaseStore,
    request: &RecoveryRequest,
    state: &RecoveryState,
    prior_boot: BootIdentity,
    recovery_boot: BootIdentity,
) -> Result<RecoveryOutcome, AviateSupervisorError> {
    let receipt = RecoveryReceipt {
        schema_version: SCHEMA_VERSION,
        run_intent_digest: request.run_intent_digest,
        spawn_intent_digest: state.intent_digest,
        process_identity_digest: state.process_digest,
        target_attestation_digest: state.target.as_ref().map(|target| target.digest),
        prior_boot_identity: prior_boot,
        recovery_boot_identity: recovery_boot,
    };
    let digest = store.publish(crate::document::RECOVERY_RECEIPT_NAME, &receipt)?;
    verify_boot_change_readback(store, &receipt, digest)?;
    Ok(RecoveryOutcome::BootChange {
        receipt_digest: digest,
    })
}

fn verify_boot_change_readback(
    store: &LeaseStore,
    expected: &RecoveryReceipt,
    expected_digest: flight_tune::Digest,
) -> Result<(), AviateSupervisorError> {
    let (actual, actual_digest): (RecoveryReceipt, _) =
        store.read(crate::document::RECOVERY_RECEIPT_NAME)?;
    if actual != *expected || actual_digest != expected_digest {
        return Err(AviateSupervisorError::invalid_document(
            "recovery receipt",
            "the recovery receipt readback changed",
        ));
    }
    Ok(())
}
