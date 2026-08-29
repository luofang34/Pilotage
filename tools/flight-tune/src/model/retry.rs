use serde::{Deserialize, Serialize};

use crate::TuneError;

/// The supported execution retry policy schema.
pub const EXECUTION_RETRY_POLICY_SCHEMA_VERSION: u16 = 1;

/// The largest retry limit any stage may declare.
const MAX_EXECUTION_RETRY_LIMIT: u32 = 8;

/// How many replacement executions one quarantined attempt may receive.
///
/// A campaign that replaces a quarantined execution states weaker evidence
/// than one that does not, because every replacement is an execution the
/// operator chose to discard. The limit is therefore part of the bar a
/// consumer states, not a detail the engine settles for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRetryPolicy {
    /// The policy schema.
    pub schema_version: u16,
    /// The largest retry index any replacement attempt may carry.
    pub execution_retry_limit: u32,
}

impl ExecutionRetryPolicy {
    /// Returns the policy that authorizes no replacement execution.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            schema_version: EXECUTION_RETRY_POLICY_SCHEMA_VERSION,
            execution_retry_limit: 0,
        }
    }

    /// Returns one policy with the stated retry limit.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the limit exceeds the supported maximum.
    pub fn with_limit(execution_retry_limit: u32) -> Result<Self, TuneError> {
        let policy = Self {
            schema_version: EXECUTION_RETRY_POLICY_SCHEMA_VERSION,
            execution_retry_limit,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Validates the schema and the declared limit.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the schema or limit is not supported.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != EXECUTION_RETRY_POLICY_SCHEMA_VERSION {
            return Err(TuneError::InvalidStage {
                detail: format!(
                    "execution retry policy schema {} is not supported",
                    self.schema_version
                ),
            });
        }
        if self.execution_retry_limit > MAX_EXECUTION_RETRY_LIMIT {
            return Err(TuneError::InvalidStage {
                detail: format!(
                    "execution retry limit {} exceeds {MAX_EXECUTION_RETRY_LIMIT}",
                    self.execution_retry_limit
                ),
            });
        }
        Ok(())
    }

    /// Reports whether a source at this retry index may receive a replacement.
    #[must_use]
    pub const fn permits_replacement(&self, source_retry_index: u32) -> bool {
        source_retry_index < self.execution_retry_limit
    }
}

#[cfg(test)]
mod tests;
