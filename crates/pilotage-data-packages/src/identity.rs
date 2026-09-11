use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{PackageError, error::invalid};

/// The SHA-256 digest of exact artifact bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentDigest(String);

impl ContentDigest {
    /// Calculate a digest from bytes.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(format!("{:x}", Sha256::digest(bytes)))
    }

    /// Return the lowercase hexadecimal digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentDigest {
    type Error = PackageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(value))
        } else {
            Err(invalid("sha256", value))
        }
    }
}

impl From<ContentDigest> for String {
    fn from(value: ContentDigest) -> Self {
        value.0
    }
}

/// An immutable release identity that is safe in a path component.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackageId(String);

impl PackageId {
    /// Return the release identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PackageId {
    type Error = PackageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if safe_component(&value) {
            Ok(Self(value))
        } else {
            Err(invalid("release_id", value))
        }
    }
}

impl From<PackageId> for String {
    fn from(value: PackageId) -> Self {
        value.0
    }
}

/// A portable path relative to an installed release directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackagePath(String);

impl PackagePath {
    /// Return the relative path with slash separators.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PackagePath {
    type Error = PackageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() <= 512 && value.split('/').all(safe_component) {
            Ok(Self(value))
        } else {
            Err(invalid("artifact_path", value))
        }
    }
}

impl From<PackagePath> for String {
    fn from(value: PackagePath) -> Self {
        value.0
    }
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
