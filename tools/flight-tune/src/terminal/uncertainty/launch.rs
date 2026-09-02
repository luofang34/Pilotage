//! The identities one uncertainty run states before it can arm.
//!
//! An executor receives these values as explicit launch arguments and hands
//! them back in its handshake. A run arms only when both sides state the
//! same identities, so a controller that loaded another artifact, another
//! seed, or another capability set cannot fly under this run's name.

use pilotage_trial::BackendCapability;
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

use super::super::invalid_terminal;
use super::EXECUTED_UNCERTAINTY_SCHEMA_VERSION;
use crate::{Digest, TuneError};

const RUN_SEED_DOMAIN: &[u8] = b"pilotage.flight-tune.executed-uncertainty-run-seed.v1\0";

/// Derives the executor run seed for one exact run intent.
///
/// Two executions of one experimental condition differ only in their retry
/// index, and the run intent covers that index. Seeding from the run intent
/// therefore gives a replacement execution a stream its quarantined source
/// never drew, while a repeated read of one run intent gives one seed.
#[must_use]
pub fn executed_run_seed(run_intent_digest: Digest) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(RUN_SEED_DOMAIN);
    hasher.update(run_intent_digest.as_bytes());
    let bytes: [u8; 32] = hasher.finalize().into();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// The exact identities a launch states and a handshake must return.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedLaunchIdentity {
    /// Launch identity schema version.
    pub schema_version: u16,
    /// The run intent that authorizes this launch.
    pub run_intent_digest: Digest,
    /// The exact bytes of the artifact the executor must load.
    pub artifact_digest: Digest,
    /// The canonical condition identity inside those bytes.
    pub condition_digest: Digest,
    /// The seed for every deterministic decision in this run.
    pub run_seed: u64,
    /// The capabilities the executor must supply, in ascending name order.
    pub required_capabilities: Vec<BackendCapability>,
    /// The trace schema the executor must speak.
    pub trace_schema_version: u16,
}

impl ExecutedLaunchIdentity {
    /// States the identities one launch will pass and check.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when an identity is absent or the capability
    /// order is not the stated one.
    pub fn new(
        run_intent_digest: Digest,
        artifact_digest: Digest,
        condition_digest: Digest,
        run_seed: u64,
        required_capabilities: Vec<BackendCapability>,
        trace_schema_version: u16,
    ) -> Result<Self, TuneError> {
        let identity = Self {
            schema_version: EXECUTED_UNCERTAINTY_SCHEMA_VERSION,
            run_intent_digest,
            artifact_digest,
            condition_digest,
            run_seed,
            required_capabilities,
            trace_schema_version,
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Rejects a launch identity that cannot name one exact execution.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the schema differs, when an identity is
    /// absent, when the artifact and condition identities are the same
    /// value, or when a capability repeats or is out of name order.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != EXECUTED_UNCERTAINTY_SCHEMA_VERSION {
            return Err(invalid_terminal("the launch identity schema changed"));
        }
        if self.run_intent_digest.is_zero()
            || self.artifact_digest.is_zero()
            || self.condition_digest.is_zero()
        {
            return Err(invalid_terminal("a launch identity is absent"));
        }
        if self.artifact_digest == self.condition_digest {
            return Err(invalid_terminal(
                "the artifact and condition identities are one value",
            ));
        }
        if self.trace_schema_version == 0 {
            return Err(invalid_terminal("a launch states no trace schema"));
        }
        if self.required_capabilities.is_empty() {
            return Err(invalid_terminal(
                "a launch identity requires no executable capability",
            ));
        }
        let mut previous: Option<&'static str> = None;
        for capability in &self.required_capabilities {
            let name = capability.as_str();
            if previous.is_some_and(|prior| prior >= name) {
                return Err(invalid_terminal(
                    "the launch capabilities are not in name order",
                ));
            }
            previous = Some(name);
        }
        Ok(())
    }

    /// Requires one returned handshake to state these exact identities.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when any returned identity differs.
    pub fn require_returned(&self, returned: &Self) -> Result<(), TuneError> {
        self.validate()?;
        if self != returned {
            return Err(invalid_terminal(
                "the executor returned other launch identities",
            ));
        }
        Ok(())
    }
}
