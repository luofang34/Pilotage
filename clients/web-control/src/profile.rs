//! The declarative control profile: schema-v1 JSON that customizes device
//! mapping and response curves ONLY.
//!
//! A profile cannot grant scopes, change authority or link-loss policy,
//! bypass neutralization, or introduce unsupported actions — the schema has
//! no field for any of those, and `deny_unknown_fields` rejects a candidate
//! that tries to add one. Compilation is the sole validation gate: a
//! candidate that does not compile can never reach [`crate::ControlRuntime`],
//! so an invalid profile disables itself before it can emit control.

use serde::Deserialize;
use sha2::{Digest, Sha256};

use pilotage_input::{AxisConfig, axis_id_for_name};

/// Schema version this runtime compiles. A candidate declaring any other
/// version is rejected before it can affect control.
pub const SCHEMA_VERSION: u32 = 1;

/// The built-in default mapping (the current LT+RStick behavior). It is
/// bytes like any other candidate: bootstrap compiles and activates it
/// through the SAME path an imported, cached, or server-restored profile
/// would use — there is no privileged default path.
pub const DEFAULT_PROFILE_BYTES: &[u8] = include_bytes!("profiles/default.json");

/// Why a candidate profile failed to compile. Every variant disables the
/// candidate; the caller keeps whatever profile was already active.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// The candidate bytes were not valid UTF-8.
    #[error("profile bytes are not valid UTF-8")]
    InvalidUtf8,
    /// The JSON did not match the schema, or carried an unsupported field
    /// (an attempt to introduce an action/authority the schema forbids).
    #[error("malformed profile JSON: {message}")]
    MalformedJson {
        /// Deserializer diagnostic.
        message: String,
    },
    /// `schema_version` was not [`SCHEMA_VERSION`].
    #[error("unsupported schema version {found}, expected {expected}")]
    UnsupportedSchemaVersion {
        /// The version the candidate declared.
        found: u32,
        /// The only version this runtime compiles.
        expected: u32,
    },
    /// A logical axis name did not resolve in the well-known table.
    #[error("unknown logical name {name:?}")]
    UnknownLogicalName {
        /// The unrecognized name.
        name: String,
    },
    /// Two roles bound the same physical input, so a tick could not decide
    /// which one an input drives.
    #[error("ambiguous binding: {detail}")]
    AmbiguousBinding {
        /// Which pair of roles collided.
        detail: &'static str,
    },
    /// An axis calibration range was degenerate (not `min < center < max`).
    #[error("malformed calibration for the {axis} axis")]
    MalformedCalibration {
        /// Which axis.
        axis: &'static str,
    },
    /// A numeric field was `NaN` or infinite.
    #[error("non-finite value in {field}")]
    NonFinite {
        /// Which field.
        field: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileDoc {
    schema_version: u32,
    revision: u32,
    id: String,
    gimbal: GimbalDoc,
    flight: FlightDoc,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GimbalDoc {
    pub(crate) modifier_button: u8,
    pub(crate) reset_button: u8,
    pub(crate) pitch: AxisConfig,
    pub(crate) yaw: AxisConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlightDoc {
    pub(crate) arm_button: u8,
    pub(crate) disarm_button: u8,
    pub(crate) left_x: usize,
    pub(crate) left_y: usize,
    pub(crate) right_x: usize,
    pub(crate) right_y: usize,
    pub(crate) trigger_left: usize,
    pub(crate) trigger_right: usize,
    pub(crate) deadzone: f32,
    pub(crate) expo: f32,
}

/// A validated, digested profile ready to activate. Carries the identity,
/// schema version, document revision, and content digest computed once at
/// compile time — the runtime never re-hashes on the hot path.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledProfile {
    id: String,
    schema_version: u32,
    revision: u32,
    digest: [u8; 32],
    pub(crate) gimbal: GimbalDoc,
    pub(crate) flight: FlightDoc,
}

impl CompiledProfile {
    /// The profile's stable identity string.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The schema version the profile was authored against.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The profile document's own revision (distinct from the runtime's
    /// session activation revision).
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// The content digest computed at compile time.
    #[must_use]
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

/// Compiles candidate profile bytes into a [`CompiledProfile`], the only way
/// to obtain one. The core accepts bytes already retrieved by its caller and
/// never learns whether they came from the built-in registry, a file,
/// IndexedDB, or an authenticated server — retrieval is not its concern.
pub struct ProfileRuntime;

impl ProfileRuntime {
    /// Validates and digests a candidate. On any error the candidate is
    /// discarded whole; the caller retains its currently active profile.
    ///
    /// # Errors
    ///
    /// Returns a typed [`ProfileError`] for invalid UTF-8/JSON, an
    /// unsupported schema version, an unknown logical name, an ambiguous
    /// binding, a malformed calibration, or a non-finite value.
    pub fn compile(candidate: &[u8]) -> Result<CompiledProfile, ProfileError> {
        let text = core::str::from_utf8(candidate).map_err(|_| ProfileError::InvalidUtf8)?;
        let doc: ProfileDoc =
            serde_json::from_str(text).map_err(|source| ProfileError::MalformedJson {
                message: source.to_string(),
            })?;
        if doc.schema_version != SCHEMA_VERSION {
            return Err(ProfileError::UnsupportedSchemaVersion {
                found: doc.schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        validate(&doc)?;
        let digest: [u8; 32] = Sha256::digest(candidate).into();
        Ok(CompiledProfile {
            id: doc.id,
            schema_version: doc.schema_version,
            revision: doc.revision,
            digest,
            gimbal: doc.gimbal,
            flight: doc.flight,
        })
    }
}

/// Full semantic validation beyond serde's structural check: logical names,
/// binding uniqueness, calibration sanity, and finiteness.
fn validate(doc: &ProfileDoc) -> Result<(), ProfileError> {
    resolve_logical(&doc.gimbal.pitch.logical)?;
    resolve_logical(&doc.gimbal.yaw.logical)?;
    check_unique(doc)?;
    check_axis("pitch", &doc.gimbal.pitch)?;
    check_axis("yaw", &doc.gimbal.yaw)?;
    check_finite("flight.deadzone", doc.flight.deadzone)?;
    check_finite("flight.expo", doc.flight.expo)?;
    Ok(())
}

fn resolve_logical(name: &str) -> Result<(), ProfileError> {
    axis_id_for_name(name)
        .map(|_| ())
        .map_err(|_| ProfileError::UnknownLogicalName {
            name: name.to_string(),
        })
}

/// No two roles may bind the same physical input, or a tick could not tell
/// which role an input drives.
fn check_unique(doc: &ProfileDoc) -> Result<(), ProfileError> {
    if doc.gimbal.modifier_button == doc.gimbal.reset_button {
        return Err(ProfileError::AmbiguousBinding {
            detail: "gimbal modifier and reset share a button",
        });
    }
    if doc.gimbal.pitch.source_index == doc.gimbal.yaw.source_index {
        return Err(ProfileError::AmbiguousBinding {
            detail: "gimbal pitch and yaw share an axis",
        });
    }
    if doc.flight.arm_button == doc.flight.disarm_button {
        return Err(ProfileError::AmbiguousBinding {
            detail: "flight arm and disarm share a button",
        });
    }
    Ok(())
}

fn check_axis(axis: &'static str, config: &AxisConfig) -> Result<(), ProfileError> {
    let cal = &config.calibration;
    check_finite(axis, config.deadzone)?;
    check_finite(axis, config.expo)?;
    check_finite(axis, cal.min)?;
    check_finite(axis, cal.center)?;
    check_finite(axis, cal.max)?;
    if !(cal.min < cal.center && cal.center < cal.max) {
        return Err(ProfileError::MalformedCalibration { axis });
    }
    Ok(())
}

fn check_finite(field: &'static str, value: f32) -> Result<(), ProfileError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ProfileError::NonFinite { field })
    }
}

#[cfg(test)]
mod tests;
