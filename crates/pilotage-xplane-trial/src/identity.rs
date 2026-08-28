use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::Digest;
use crate::error::XPlaneTrialError;
use crate::protocol::Hello;

/// One expected host file and its pinned identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedArtifact {
    path: PathBuf,
    digest: Digest,
}

impl ExpectedArtifact {
    /// Creates one expected file identity.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, digest: Digest) -> Self {
        Self {
            path: path.into(),
            digest,
        }
    }

    /// Returns the expected path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the expected content digest.
    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }
}

/// The pinned identity required before one X-Plane trial session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedXPlaneIdentity {
    /// The active aircraft file.
    pub aircraft: ExpectedArtifact,
    /// The Pilotage trial plugin.
    pub trial_plugin: ExpectedArtifact,
    /// The flight-controller bridge plugin.
    pub bridge_plugin: ExpectedArtifact,
    /// The bridge configuration file.
    pub bridge_config: ExpectedArtifact,
    /// The source identity embedded in the loaded trial plugin.
    pub trial_source_build_id: String,
    /// The complete simulator model contract.
    pub simulator_model_digest: Digest,
}

/// The verified identity of one running X-Plane session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedXPlaneIdentity {
    /// The trial protocol version.
    pub protocol_version: u32,
    /// The running X-Plane version number.
    pub xplane_version: u32,
    /// The running plugin SDK version number.
    pub sdk_version: u32,
    /// The X-Plane host application identity.
    pub host_application_id: u32,
    /// The trial plugin source-build identity.
    pub trial_source_build_id: String,
    /// The active aircraft file digest.
    pub aircraft_digest: Digest,
    /// The loaded trial plugin file digest.
    pub trial_plugin_digest: Digest,
    /// The loaded flight-controller bridge file digest.
    pub bridge_plugin_digest: Digest,
    /// The active bridge configuration digest.
    pub bridge_config_digest: Digest,
    /// The complete simulator model contract digest.
    pub simulator_model_digest: Digest,
    /// The complete verified session binding digest.
    pub binding_digest: Digest,
}

impl VerifiedXPlaneIdentity {
    /// Recalculates the complete binding after identity fields are populated.
    pub fn refresh_binding_digest(&mut self) {
        self.binding_digest = binding_digest(self);
    }

    /// Returns true when the complete session binding can be recomputed.
    #[must_use]
    pub fn binding_is_valid(&self) -> bool {
        !self.binding_digest.is_zero() && self.binding_digest == binding_digest(self)
    }
}

/// A capability that exists only after runtime file verification.
#[derive(Debug)]
pub struct VerifiedXPlaneBinding {
    identity: VerifiedXPlaneIdentity,
}

impl VerifiedXPlaneBinding {
    /// Returns the verified runtime identity.
    #[must_use]
    pub const fn identity(&self) -> &VerifiedXPlaneIdentity {
        &self.identity
    }
}

pub(crate) fn binding(identity: VerifiedXPlaneIdentity) -> VerifiedXPlaneBinding {
    VerifiedXPlaneBinding { identity }
}

pub(crate) fn verify_blocking(
    expected: &ExpectedXPlaneIdentity,
    hello: &Hello,
) -> Result<VerifiedXPlaneIdentity, XPlaneTrialError> {
    if expected.simulator_model_digest.is_zero() {
        return Err(XPlaneTrialError::ZeroModelDigest);
    }
    if expected.trial_source_build_id.is_empty()
        || expected.trial_source_build_id != hello.source_build_id
    {
        return Err(XPlaneTrialError::TrialSourceBuild {
            expected: expected.trial_source_build_id.clone(),
            actual: hello.source_build_id.clone(),
        });
    }
    if hello.bridge_build_digest != expected.bridge_plugin.digest {
        return Err(XPlaneTrialError::LoadedBridgeDigest {
            expected: expected.bridge_plugin.digest,
            actual: hello.bridge_build_digest,
        });
    }
    verify_artifact_blocking("aircraft", &expected.aircraft, &hello.aircraft_path)?;
    verify_artifact_blocking(
        "trial plugin",
        &expected.trial_plugin,
        &hello.trial_plugin_path,
    )?;
    let actual_bridge_plugin = verify_artifact_blocking(
        "bridge plugin",
        &expected.bridge_plugin,
        &hello.bridge_plugin_path,
    )?;
    let actual_bridge_config = actual_bridge_plugin
        .parent()
        .map_or_else(PathBuf::new, |path| path.join("config.ini"));
    verify_artifact_blocking(
        "bridge configuration",
        &expected.bridge_config,
        &actual_bridge_config,
    )?;
    let mut identity = VerifiedXPlaneIdentity {
        protocol_version: hello.protocol_version,
        xplane_version: hello.xplane_version,
        sdk_version: hello.sdk_version,
        host_application_id: hello.host_application_id,
        trial_source_build_id: hello.source_build_id.clone(),
        aircraft_digest: expected.aircraft.digest,
        trial_plugin_digest: expected.trial_plugin.digest,
        bridge_plugin_digest: expected.bridge_plugin.digest,
        bridge_config_digest: expected.bridge_config.digest,
        simulator_model_digest: expected.simulator_model_digest,
        binding_digest: Digest::from_bytes([0; 32]),
    };
    identity.refresh_binding_digest();
    Ok(identity)
}

fn verify_artifact_blocking(
    artifact: &'static str,
    expected: &ExpectedArtifact,
    actual_path: &Path,
) -> Result<PathBuf, XPlaneTrialError> {
    let expected_path = canonical_path(artifact, &expected.path)?;
    let actual_path = runtime_path(artifact, actual_path, &expected_path)?;
    if expected_path != actual_path {
        return Err(XPlaneTrialError::ArtifactPath {
            artifact,
            expected: expected_path,
            actual: actual_path,
        });
    }
    let actual = file_digest_blocking(artifact, &actual_path)?;
    if actual != expected.digest {
        return Err(XPlaneTrialError::ArtifactDigest {
            artifact,
            expected: expected.digest,
            actual,
        });
    }
    Ok(actual_path)
}

fn runtime_path(
    artifact: &'static str,
    path: &Path,
    expected: &Path,
) -> Result<PathBuf, XPlaneTrialError> {
    if path.is_absolute() {
        return canonical_path(artifact, path);
    }
    let Some(text) = path.to_str() else {
        return canonical_path(artifact, path);
    };
    let Some((_, components)) = text.split_once(':') else {
        return canonical_path(artifact, path);
    };
    let actual = components.split(':').collect::<Vec<_>>();
    let expected_components = expected
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    if actual == expected_components {
        Ok(expected.to_path_buf())
    } else {
        Err(XPlaneTrialError::ArtifactPath {
            artifact,
            expected: expected.to_path_buf(),
            actual: path.to_path_buf(),
        })
    }
}

fn canonical_path(artifact: &'static str, path: &Path) -> Result<PathBuf, XPlaneTrialError> {
    std::fs::canonicalize(path).map_err(|source| XPlaneTrialError::ArtifactRead {
        artifact,
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn file_digest_blocking(
    artifact: &'static str,
    path: &Path,
) -> Result<Digest, XPlaneTrialError> {
    let mut file = File::open(path).map_err(|source| XPlaneTrialError::ArtifactRead {
        artifact,
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let size = file
            .read(&mut buffer)
            .map_err(|source| XPlaneTrialError::ArtifactRead {
                artifact,
                path: path.to_path_buf(),
                source,
            })?;
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    Ok(Digest::from_bytes(hasher.finalize().into()))
}

fn binding_digest(identity: &VerifiedXPlaneIdentity) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"pilotage.xplane.verified-session.v1\0");
    for value in [
        identity.protocol_version,
        identity.xplane_version,
        identity.sdk_version,
        identity.host_application_id,
    ] {
        hasher.update(value.to_le_bytes());
    }
    hasher.update((identity.trial_source_build_id.len() as u64).to_le_bytes());
    hasher.update(identity.trial_source_build_id.as_bytes());
    for digest in [
        identity.aircraft_digest,
        identity.trial_plugin_digest,
        identity.bridge_plugin_digest,
        identity.bridge_config_digest,
        identity.simulator_model_digest,
    ] {
        hasher.update(digest.as_bytes());
    }
    Digest::from_bytes(hasher.finalize().into())
}
