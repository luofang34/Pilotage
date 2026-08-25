use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::AviateSupervisorError;
use crate::document::TargetProcessContract;
use crate::document::{AnchoredDirectory, AnchoredExecutable};

pub(crate) const BOOTSTRAP_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupervisorBootstrap {
    pub(crate) schema_version: u16,
    pub(crate) run_intent_digest: flight_tune::Digest,
    pub(crate) release_secret_digest: flight_tune::Digest,
    pub(crate) supervisor_executable: AnchoredExecutable,
    pub(crate) target_executable: AnchoredExecutable,
    pub(crate) artifact_root: AnchoredDirectory,
    pub(crate) runtime_root: AnchoredDirectory,
    pub(crate) target_arguments: Vec<String>,
    pub(crate) target_environment: BTreeMap<String, String>,
    pub(crate) target_process_contract: TargetProcessContract,
    pub(crate) target_current_directory: AnchoredDirectory,
    pub(crate) startup_timeout_millis: u64,
    pub(crate) cleanup_timeout_millis: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GateConfig {
    pub(super) schema_version: u16,
    pub(super) release_secret_digest: flight_tune::Digest,
    pub(super) target_executable: AnchoredExecutable,
    pub(super) artifact_root: AnchoredDirectory,
    pub(super) target_arguments: Vec<String>,
    pub(super) target_environment: BTreeMap<String, String>,
    pub(super) target_environment_digest: flight_tune::Digest,
    pub(super) target_current_directory: AnchoredDirectory,
    pub(super) target_argv_digest: flight_tune::Digest,
    pub(super) target_process_contract: TargetProcessContract,
}

pub(super) fn validate_bootstrap(
    bootstrap: &SupervisorBootstrap,
) -> Result<(), AviateSupervisorError> {
    if bootstrap.schema_version != BOOTSTRAP_SCHEMA_VERSION
        || bootstrap.run_intent_digest.is_zero()
        || bootstrap.release_secret_digest.is_zero()
        || bootstrap.startup_timeout_millis == 0
        || bootstrap.cleanup_timeout_millis == 0
    {
        return Err(AviateSupervisorError::invalid_request(
            "the supervisor bootstrap is incomplete",
        ));
    }
    validate_arguments(&bootstrap.target_arguments)?;
    validate_environment(&bootstrap.target_environment)
}

pub(super) fn validate_gate_config(config: &GateConfig) -> Result<(), AviateSupervisorError> {
    if config.schema_version != BOOTSTRAP_SCHEMA_VERSION
        || config.release_secret_digest.is_zero()
        || config.target_environment_digest != digest_environment(&config.target_environment)
    {
        return Err(AviateSupervisorError::invalid_request(
            "the launch-gate configuration is incomplete",
        ));
    }
    validate_arguments(&config.target_arguments)?;
    validate_environment(&config.target_environment)
}

pub(crate) fn digest_environment(environment: &BTreeMap<String, String>) -> flight_tune::Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"flight-tune-aviate-env-v1\0");
    for (name, value) in environment {
        hash_field(&mut hasher, name.as_bytes());
        hash_field(&mut hasher, value.as_bytes());
    }
    flight_tune::Digest::from_bytes(hasher.finalize().into())
}

fn validate_arguments(arguments: &[String]) -> Result<(), AviateSupervisorError> {
    if arguments.len() > 256 || arguments.iter().any(|value| value.contains('\0')) {
        return Err(AviateSupervisorError::invalid_request(
            "the target argument vector is invalid",
        ));
    }
    Ok(())
}

fn validate_environment(
    environment: &BTreeMap<String, String>,
) -> Result<(), AviateSupervisorError> {
    if environment.len() > 256
        || environment.iter().any(|(name, value)| {
            name.is_empty() || name.contains(['=', '\0']) || value.contains('\0')
        })
    {
        return Err(AviateSupervisorError::invalid_request(
            "the explicit target environment is invalid",
        ));
    }
    Ok(())
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}
