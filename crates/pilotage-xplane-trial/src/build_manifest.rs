use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Digest, XPlaneTrialError};

const BUILD_MANIFEST_SCHEMA_VERSION: u16 = 1;
const MAXIMUM_BUILD_MANIFEST_BYTES: usize = 4 * 1024;

/// The identities embedded when the trial and bridge plugins are built.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialPluginBuildManifest {
    /// The build-manifest schema version.
    pub schema_version: u16,
    /// The source identity embedded in the trial plugin.
    pub trial_source_build_id: String,
    /// The bridge binary identity embedded in the trial plugin.
    pub bridge_plugin_digest: Digest,
}

impl TrialPluginBuildManifest {
    /// Reads and validates an installed build manifest.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or its content is not
    /// the strict build-manifest contract.
    pub fn from_json_file_blocking(path: &Path) -> Result<Self, XPlaneTrialError> {
        let bytes = std::fs::read(path).map_err(|source| XPlaneTrialError::BuildManifestRead {
            path: path.to_path_buf(),
            source,
        })?;
        if bytes.len() > MAXIMUM_BUILD_MANIFEST_BYTES {
            return Err(XPlaneTrialError::BuildManifestTooLarge {
                path: path.to_path_buf(),
                size: bytes.len(),
            });
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|source| {
            XPlaneTrialError::BuildManifestDecode {
                path: path.to_path_buf(),
                source,
            }
        })?;
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), XPlaneTrialError> {
        if self.schema_version != BUILD_MANIFEST_SCHEMA_VERSION {
            return invalid("the build-manifest schema version does not match");
        }
        if self.trial_source_build_id.is_empty() || self.trial_source_build_id.len() > 256 {
            return invalid("the trial source build identity is not valid");
        }
        if self.bridge_plugin_digest.is_zero() {
            return invalid("the bridge plugin digest is zero");
        }
        Ok(())
    }
}

fn invalid<T>(detail: impl Into<String>) -> Result<T, XPlaneTrialError> {
    Err(XPlaneTrialError::InvalidBuildManifest {
        detail: detail.into(),
    })
}
