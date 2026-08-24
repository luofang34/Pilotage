use serde::{Deserialize, Serialize};

use crate::{ValidationError, validation::duration};

const MAX_SOURCE_DELAY_NS: u64 = 100_000_000;

/// A deterministic update-jitter model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DelayJitter {
    /// Do not add timing jitter.
    None,
    /// Hold one seeded extra delay for each fixed interval.
    SampleAndHold {
        /// Maximum additional delay in nanoseconds.
        maximum_delay_ns: u64,
        /// Interval between deterministic delay values in nanoseconds.
        interval_ns: u64,
    },
}

/// Source timing perturbation for one condition set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingCondition {
    /// Fixed estimate age in nanoseconds.
    pub estimate_delay_ns: u64,
    /// Seeded additional update delay.
    pub update_jitter: DelayJitter,
}

impl TimingCondition {
    /// Returns a condition with no source timing perturbation.
    #[must_use]
    pub const fn nominal() -> Self {
        Self {
            estimate_delay_ns: 0,
            update_jitter: DelayJitter::None,
        }
    }

    pub(super) fn validate(self) -> Result<(), ValidationError> {
        let maximum = self
            .estimate_delay_ns
            .saturating_add(self.update_jitter.maximum_delay_ns());
        if maximum > MAX_SOURCE_DELAY_NS {
            return Err(ValidationError::OutOfRange {
                field: "condition_set.timing.maximum_source_delay_ns".to_owned(),
                actual: maximum as f64,
                minimum: 0.0,
                maximum: MAX_SOURCE_DELAY_NS as f64,
            });
        }
        self.update_jitter.validate()
    }

    pub(super) fn delay_ns(self, condition_seed: u64, run_seed: u64, elapsed_ns: u64) -> u64 {
        self.estimate_delay_ns.saturating_add(
            self.update_jitter
                .sample(condition_seed ^ run_seed.rotate_left(29), elapsed_ns),
        )
    }
}

impl DelayJitter {
    const fn maximum_delay_ns(self) -> u64 {
        match self {
            Self::None => 0,
            Self::SampleAndHold {
                maximum_delay_ns, ..
            } => maximum_delay_ns,
        }
    }

    fn validate(self) -> Result<(), ValidationError> {
        match self {
            Self::None => Ok(()),
            Self::SampleAndHold {
                maximum_delay_ns,
                interval_ns,
            } => {
                duration(
                    "condition_set.timing.update_jitter.maximum_delay_ns",
                    maximum_delay_ns,
                )?;
                duration(
                    "condition_set.timing.update_jitter.interval_ns",
                    interval_ns,
                )
            }
        }
    }

    fn sample(self, seed: u64, elapsed_ns: u64) -> u64 {
        match self {
            Self::None => 0,
            Self::SampleAndHold {
                maximum_delay_ns,
                interval_ns,
            } => {
                if interval_ns == 0 || maximum_delay_ns == 0 {
                    return 0;
                }
                let interval = elapsed_ns / interval_ns;
                splitmix64(seed ^ interval.wrapping_mul(0x9e37_79b9_7f4a_7c15))
                    % maximum_delay_ns.saturating_add(1)
            }
        }
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
