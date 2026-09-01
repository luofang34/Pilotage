//! The one content-addressed artifact a non-nominal run launches with.
//!
//! The executor receives the artifact path, the identity of its exact bytes,
//! the canonical condition identity inside them, the run seed, and the
//! capability set the condition needs. It receives them as explicit launch
//! arguments, so no part of the run's uncertainty can arrive through a name
//! the launcher never stated.
//!
//! Nothing is written before the backend has been asked whether it can
//! execute the condition at all. A backend that declares no uncertainty
//! capability refuses the condition before the artifact exists.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use flight_tune::{
    ConditionAdmission, ConditionSet, Digest, ExecutedLaunchIdentity,
    ExecutedUncertaintyDeclaration, executed_run_seed,
};
use sha2::{Digest as ShaDigest, Sha256};

use super::error::AviateConditionError;

/// The trace protocol version this launch speaks.
pub const TUNING_TRACE_SCHEMA_VERSION: u16 = 3;

const ARTIFACT_NAME: &str = "condition.json";
const MANIFEST_NAME: &str = "run-manifest.toml";

/// One prepared non-nominal condition launch.
#[derive(Clone, Debug)]
pub struct ConditionLaunch {
    identity: ExecutedLaunchIdentity,
    declaration: ExecutedUncertaintyDeclaration,
    artifact_path: PathBuf,
    manifest_path: PathBuf,
    trace_endpoint: SocketAddr,
}

impl ConditionLaunch {
    /// Prepares one condition for launch after the backend admits it.
    ///
    /// The admission runs first, so a backend that cannot execute the
    /// condition refuses it before any file exists.
    ///
    /// # Errors
    ///
    /// Returns [`AviateConditionError`] when the backend cannot execute the
    /// condition, when the condition is nominal, when the artifact cannot be
    /// encoded, or when it cannot be made durable.
    pub fn prepare_blocking(
        condition: &ConditionSet,
        admission: &ConditionAdmission,
        run_intent_digest: Digest,
        directory: &Path,
        trace_endpoint: SocketAddr,
    ) -> Result<Self, AviateConditionError> {
        admission
            .prepare(condition)
            .map_err(|source| unsupported(condition, source))?;
        let run_seed = executed_run_seed(run_intent_digest);
        let bytes = artifact_bytes(condition)?;
        let artifact_digest = Digest::from_bytes(Sha256::digest(&bytes).into());
        let declaration =
            ExecutedUncertaintyDeclaration::from_condition(condition, artifact_digest, run_seed)
                .map_err(|source| AviateConditionError::Evidence { source })?;
        if declaration.is_nominal() {
            return Err(AviateConditionError::protocol(
                "a nominal condition needs no launch artifact",
            ));
        }
        let identity = ExecutedLaunchIdentity::new(
            run_intent_digest,
            artifact_digest,
            declaration.condition_digest,
            run_seed,
            declaration.required_capabilities.clone(),
            TUNING_TRACE_SCHEMA_VERSION,
        )
        .map_err(|source| AviateConditionError::Evidence { source })?;
        let artifact_path = directory.join(ARTIFACT_NAME);
        write_artifact_blocking(&artifact_path, &bytes)?;
        Ok(Self {
            identity,
            declaration,
            artifact_path,
            manifest_path: directory.join(MANIFEST_NAME),
            trace_endpoint,
        })
    }

    /// Returns the complete condition arguments, after argument zero.
    ///
    /// The executor accepts the five condition arguments together or not at
    /// all, and refuses a condition run that names no trace path or run
    /// manifest, so the whole set is one value.
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        vec![
            "--condition-artifact".to_owned(),
            self.artifact_path.to_string_lossy().into_owned(),
            "--condition-artifact-sha256".to_owned(),
            self.identity.artifact_digest.to_string(),
            "--condition-digest".to_owned(),
            self.identity.condition_digest.to_string(),
            "--run-seed".to_owned(),
            self.identity.run_seed.to_string(),
            "--required-perturbation-capabilities".to_owned(),
            capability_argument(&self.identity),
            "--tuning-trace-endpoint".to_owned(),
            self.trace_endpoint.to_string(),
            "--run-manifest".to_owned(),
            self.manifest_path.to_string_lossy().into_owned(),
        ]
    }

    /// Returns the identities the executor must return before arming.
    #[must_use]
    pub const fn identity(&self) -> &ExecutedLaunchIdentity {
        &self.identity
    }

    /// Returns what this launch declared the run would execute.
    #[must_use]
    pub const fn declaration(&self) -> &ExecutedUncertaintyDeclaration {
        &self.declaration
    }

    /// Returns the path of the exact artifact the executor must load.
    #[must_use]
    pub fn artifact_path(&self) -> &Path {
        &self.artifact_path
    }

    /// Returns the path the executor must write its run manifest to.
    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }
}

/// Joins the capability names the executor must supply.
///
/// The executor parses this value without trimming and requires it to be
/// strictly ascending by name, so the separator carries no space and the
/// order is the declared one.
fn capability_argument(identity: &ExecutedLaunchIdentity) -> String {
    identity
        .required_capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// Encodes the exact bytes the artifact file carries.
///
/// The file is the canonical condition document and one closing line end.
/// The line end keeps the identity of the bytes separate from the identity
/// of the document inside them, so an argument that names one cannot pass
/// for the other.
fn artifact_bytes(condition: &ConditionSet) -> Result<Vec<u8>, AviateConditionError> {
    let mut bytes = condition
        .to_canonical_json()
        .map_err(|source| AviateConditionError::Encode { source })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_artifact_blocking(path: &Path, bytes: &[u8]) -> Result<(), AviateConditionError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| AviateConditionError::Artifact {
            operation: "create the condition artifact directory",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, bytes).map_err(|source| AviateConditionError::Artifact {
        operation: "write the condition artifact",
        path: path.to_path_buf(),
        source,
    })
}

fn unsupported(
    condition: &ConditionSet,
    source: flight_tune::ScenarioRuntimeError,
) -> AviateConditionError {
    match source {
        flight_tune::ScenarioRuntimeError::UnsupportedCondition { source, .. } => {
            AviateConditionError::Unsupported {
                condition: condition.id.clone(),
                source,
            }
        }
        other => AviateConditionError::protocol(other.to_string()),
    }
}
