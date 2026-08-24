//! Immutable identity data for one trial run.

use serde::{Deserialize, Serialize};

use crate::{
    CodecError, Digest, MAX_CLOCK_MAPPINGS, MAX_RUN_IDENTITY_BYTES, MAX_TEXT_BYTES,
    RUN_IDENTITY_SCHEMA_VERSION, ValidationError, canonical,
    validation::{count, digest, schema, text},
};

/// The identity of one immutable artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    /// The stable artifact identifier.
    pub id: String,
    /// The artifact revision.
    pub revision: String,
    /// The artifact content digest.
    pub digest: Digest,
}

impl ArtifactIdentity {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        text(&format!("{field}.id"), &self.id, MAX_TEXT_BYTES)?;
        text(&format!("{field}.revision"), &self.revision, MAX_TEXT_BYTES)?;
        digest(&format!("{field}.digest"), self.digest)
    }
}

/// The identity of one scenario revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioIdentity {
    /// The stable scenario identifier.
    pub id: String,
    /// The scenario revision number.
    pub revision: u32,
    /// The digest of the canonical scenario JSON.
    pub digest: Digest,
}

impl ScenarioIdentity {
    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        text("run.scenario.id", &self.id, MAX_TEXT_BYTES)?;
        digest("run.scenario.digest", self.digest)
    }
}

/// A clock domain in a trial trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockDomain {
    /// The input device clock.
    Device,
    /// The control client clock.
    Client,
    /// The recorder host clock.
    Recorder,
    /// The adapter clock.
    Adapter,
    /// The flight controller clock.
    FlightController,
    /// The simulator clock.
    Simulator,
}

/// The quality of one clock mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockMappingQuality {
    /// The source supplies an exact mapping.
    Exact,
    /// Measurement supplies an estimated mapping.
    Estimated,
    /// The mapping is not usable for timing analysis.
    Unusable,
}

/// A mapping from one source clock epoch to the recorder clock.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockMapping {
    /// The source clock.
    pub from: ClockDomain,
    /// The target clock. This value must be [`ClockDomain::Recorder`].
    pub to: ClockDomain,
    /// The source clock epoch.
    pub source_epoch: u64,
    /// The source time at the mapping anchor.
    pub source_anchor_ns: u64,
    /// The recorder time at the mapping anchor.
    pub recorder_anchor_ns: u64,
    /// The identity-bearing recorder-rate numerator for one source-clock unit.
    pub rate_numerator: u64,
    /// The identity-bearing recorder-rate denominator for one source-clock unit.
    pub rate_denominator: u64,
    /// The first source time for which this mapping is valid.
    pub valid_from_source_ns: u64,
    /// The last source time for which this mapping is valid.
    pub valid_until_source_ns: u64,
    /// The maximum mapping uncertainty.
    pub uncertainty_ns: u64,
    /// The mapping quality.
    pub quality: ClockMappingQuality,
}

/// The bounded recorder time for one mapped source time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecorderTimeInterval {
    /// The first possible recorder time.
    pub earliest_ns: u64,
    /// The last possible recorder time.
    pub latest_ns: u64,
}

impl ClockMapping {
    /// Maps a valid source time to a checked recorder time interval.
    ///
    /// The interval contains both adjacent integer nanoseconds when the rate
    /// produces a fractional recorder time.
    #[must_use]
    pub fn mapped_recorder_interval(&self, source_time_ns: u64) -> Option<RecorderTimeInterval> {
        if self.quality == ClockMappingQuality::Unusable
            || !self.has_valid_shape()
            || !(self.valid_from_source_ns..=self.valid_until_source_ns).contains(&source_time_ns)
        {
            return None;
        }
        self.checked_recorder_interval(source_time_ns)
    }

    fn has_valid_shape(&self) -> bool {
        self.from != ClockDomain::Recorder
            && self.to == ClockDomain::Recorder
            && self.rate_numerator != 0
            && self.rate_denominator != 0
            && self.valid_from_source_ns <= self.valid_until_source_ns
            && (self.valid_from_source_ns..=self.valid_until_source_ns)
                .contains(&self.source_anchor_ns)
            && (self.quality != ClockMappingQuality::Exact || self.uncertainty_ns == 0)
            && self
                .checked_recorder_interval(self.valid_from_source_ns)
                .is_some()
            && self
                .checked_recorder_interval(self.valid_until_source_ns)
                .is_some()
    }

    fn checked_recorder_interval(&self, source_time_ns: u64) -> Option<RecorderTimeInterval> {
        if self.rate_numerator == 0 || self.rate_denominator == 0 {
            return None;
        }
        let source_delta = source_time_ns.abs_diff(self.source_anchor_ns);
        let scaled = u128::from(source_delta)
            .checked_mul(u128::from(self.rate_numerator))?
            .checked_div(u128::from(self.rate_denominator))?;
        let remainder = u128::from(source_delta)
            .checked_mul(u128::from(self.rate_numerator))?
            .checked_rem(u128::from(self.rate_denominator))?;
        let lower_delta = u64::try_from(scaled).ok()?;
        let upper_delta = u64::try_from(scaled.checked_add(u128::from(remainder != 0))?).ok()?;
        if source_time_ns >= self.source_anchor_ns {
            let earliest_ns = self.recorder_anchor_ns.checked_add(lower_delta)?;
            let latest_ns = self.recorder_anchor_ns.checked_add(upper_delta)?;
            self.add_uncertainty(earliest_ns, latest_ns)
        } else {
            let earliest_ns = self.recorder_anchor_ns.checked_sub(upper_delta)?;
            let latest_ns = self.recorder_anchor_ns.checked_sub(lower_delta)?;
            self.add_uncertainty(earliest_ns, latest_ns)
        }
    }

    fn add_uncertainty(&self, earliest_ns: u64, latest_ns: u64) -> Option<RecorderTimeInterval> {
        Some(RecorderTimeInterval {
            earliest_ns: earliest_ns.checked_sub(self.uncertainty_ns)?,
            latest_ns: latest_ns.checked_add(self.uncertainty_ns)?,
        })
    }
}

/// The immutable identity of one trial run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunIdentity {
    /// The run identity schema version.
    pub schema_version: u16,
    /// The unique run identifier.
    pub run_id: String,
    /// The code build identity.
    pub code_build: ArtifactIdentity,
    /// The vehicle adapter identity.
    pub vehicle_adapter: ArtifactIdentity,
    /// The vehicle adapter capabilities digest.
    pub adapter_capabilities_digest: Digest,
    /// The simulator backend capabilities digest.
    pub backend_capabilities_digest: Digest,
    /// The input device profile identity.
    pub device_profile: ArtifactIdentity,
    /// The control scheme identity.
    pub control_scheme: ArtifactIdentity,
    /// The control feel candidate identity.
    pub control_feel_candidate: ArtifactIdentity,
    /// The flight controller candidate identity.
    pub flight_controller_candidate: ArtifactIdentity,
    /// The simulator backend identity.
    pub simulator_backend: ArtifactIdentity,
    /// The simulator build identity.
    pub simulator: ArtifactIdentity,
    /// The simulated vehicle model identity.
    pub vehicle_model: ArtifactIdentity,
    /// The environmental condition set identity.
    pub condition_set: ArtifactIdentity,
    /// The scenario identity.
    pub scenario: ScenarioIdentity,
    /// The deterministic scenario seed.
    pub seed: u64,
    /// The repetition number for this run.
    pub repetition: u32,
    /// The identity-bearing ordered list of clock mappings.
    ///
    /// Mapping order and an equivalent unreduced rate are distinct identities.
    pub clock_mappings: Vec<ClockMapping>,
}

impl RunIdentity {
    /// Decodes and validates a run identity JSON document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, CodecError> {
        let value: Self = canonical::decode("run identity", bytes, MAX_RUN_IDENTITY_BYTES)?;
        value.validate()?;
        Ok(value)
    }

    /// Validates all required identity data.
    pub fn validate(&self) -> Result<(), ValidationError> {
        schema(
            "run identity",
            self.schema_version,
            RUN_IDENTITY_SCHEMA_VERSION,
        )?;
        text("run.run_id", &self.run_id, MAX_TEXT_BYTES)?;
        self.validate_artifacts()?;
        digest(
            "run.adapter_capabilities_digest",
            self.adapter_capabilities_digest,
        )?;
        digest(
            "run.backend_capabilities_digest",
            self.backend_capabilities_digest,
        )?;
        self.scenario.validate()?;
        self.validate_clock_mappings()
    }

    /// Encodes canonical compact JSON after validation.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, CodecError> {
        self.validate()?;
        canonical::encode("run identity", self, MAX_RUN_IDENTITY_BYTES)
    }

    /// Calculates the canonical run identity digest.
    pub fn canonical_digest(&self) -> Result<Digest, CodecError> {
        self.to_canonical_json()
            .map(|bytes| canonical::digest(&bytes))
    }

    fn validate_artifacts(&self) -> Result<(), ValidationError> {
        self.code_build.validate("run.code_build")?;
        self.vehicle_adapter.validate("run.vehicle_adapter")?;
        self.device_profile.validate("run.device_profile")?;
        self.control_scheme.validate("run.control_scheme")?;
        self.control_feel_candidate
            .validate("run.control_feel_candidate")?;
        self.flight_controller_candidate
            .validate("run.flight_controller_candidate")?;
        self.simulator_backend.validate("run.simulator_backend")?;
        self.simulator.validate("run.simulator")?;
        self.vehicle_model.validate("run.vehicle_model")?;
        self.condition_set.validate("run.condition_set")
    }

    fn validate_clock_mappings(&self) -> Result<(), ValidationError> {
        count(
            "run.clock_mappings",
            self.clock_mappings.len(),
            MAX_CLOCK_MAPPINGS,
        )?;
        for (index, mapping) in self.clock_mappings.iter().enumerate() {
            self.validate_clock_mapping(index, mapping)?;
        }
        Ok(())
    }

    fn validate_clock_mapping(
        &self,
        index: usize,
        mapping: &ClockMapping,
    ) -> Result<(), ValidationError> {
        if mapping.from == ClockDomain::Recorder || mapping.to != ClockDomain::Recorder {
            return invalid_clock_mapping(index, "each source must map directly to the recorder");
        }
        if mapping.rate_numerator == 0 || mapping.rate_denominator == 0 {
            return invalid_clock_mapping(index, "the clock rate ratio must be greater than zero");
        }
        if mapping.valid_from_source_ns > mapping.valid_until_source_ns {
            return invalid_clock_mapping(index, "the validity interval is reversed");
        }
        if !(mapping.valid_from_source_ns..=mapping.valid_until_source_ns)
            .contains(&mapping.source_anchor_ns)
        {
            return invalid_clock_mapping(
                index,
                "the source anchor is outside the validity interval",
            );
        }
        if mapping.quality == ClockMappingQuality::Exact && mapping.uncertainty_ns != 0 {
            return invalid_clock_mapping(index, "an exact mapping must have zero uncertainty");
        }
        if mapping
            .checked_recorder_interval(mapping.valid_from_source_ns)
            .is_none()
            || mapping
                .checked_recorder_interval(mapping.valid_until_source_ns)
                .is_none()
        {
            return invalid_clock_mapping(index, "the mapping interval exceeds the recorder clock");
        }
        let duplicate = self.clock_mappings[..index]
            .iter()
            .any(|item| item.from == mapping.from && item.source_epoch == mapping.source_epoch);
        if duplicate {
            return invalid_clock_mapping(index, "the source clock epoch occurs more than once");
        }
        Ok(())
    }

    pub(crate) fn validate_stage_clock(
        &self,
        field: &str,
        clock: ClockDomain,
        epoch: u64,
        source_time_ns: Option<u64>,
        recorder_receive_ns: u64,
    ) -> Result<(), ValidationError> {
        let Some(time_ns) = source_time_ns else {
            return if clock == ClockDomain::Recorder {
                self.validate_clock_epoch(field, clock, epoch)
            } else {
                Ok(())
            };
        };
        let mapped = self.map_clock_time(field, clock, epoch, time_ns)?;
        if mapped.latest_ns > recorder_receive_ns {
            return invalid_stage_stamp(field, "the mapped source time is after the receive time");
        }
        Ok(())
    }

    pub(crate) fn validate_sample_clock(
        &self,
        field: &str,
        clock: ClockDomain,
        epoch: u64,
        source_time_ns: u64,
        recorder_sample_ns: u64,
    ) -> Result<(), ValidationError> {
        let mapped = self.map_clock_time(field, clock, epoch, source_time_ns)?;
        if !(mapped.earliest_ns..=mapped.latest_ns).contains(&recorder_sample_ns) {
            return invalid_clock_observation(
                field,
                "the mapped time differs from the recorder sample time",
            );
        }
        Ok(())
    }

    fn validate_clock_epoch(
        &self,
        field: &str,
        clock: ClockDomain,
        epoch: u64,
    ) -> Result<(), ValidationError> {
        if clock == ClockDomain::Recorder {
            return if epoch == 0 {
                Ok(())
            } else {
                invalid_clock_observation(field, "the recorder clock epoch must be zero")
            };
        }
        self.clock_mapping(field, clock, epoch).map(|_| ())
    }

    pub(crate) fn map_clock_time(
        &self,
        field: &str,
        clock: ClockDomain,
        epoch: u64,
        source_time_ns: u64,
    ) -> Result<RecorderTimeInterval, ValidationError> {
        if clock == ClockDomain::Recorder {
            self.validate_clock_epoch(field, clock, epoch)?;
            return Ok(RecorderTimeInterval {
                earliest_ns: source_time_ns,
                latest_ns: source_time_ns,
            });
        }
        let mapping = self.clock_mapping(field, clock, epoch)?;
        validate_mapping_use(field, clock, epoch, source_time_ns, mapping)
    }

    fn clock_mapping(
        &self,
        field: &str,
        clock: ClockDomain,
        epoch: u64,
    ) -> Result<&ClockMapping, ValidationError> {
        self.clock_mappings
            .iter()
            .find(|mapping| mapping.from == clock && mapping.source_epoch == epoch)
            .ok_or_else(|| ValidationError::MissingClockMapping {
                field: field.to_owned(),
                clock: format!("{clock:?}"),
                epoch,
            })
    }
}

fn validate_mapping_use(
    field: &str,
    clock: ClockDomain,
    epoch: u64,
    source_time_ns: u64,
    mapping: &ClockMapping,
) -> Result<RecorderTimeInterval, ValidationError> {
    if mapping.quality == ClockMappingQuality::Unusable {
        return Err(ValidationError::UnusableClockMapping {
            field: field.to_owned(),
            clock: format!("{clock:?}"),
            epoch,
        });
    }
    if !(mapping.valid_from_source_ns..=mapping.valid_until_source_ns).contains(&source_time_ns) {
        return Err(ValidationError::ClockTimeOutsideMapping {
            field: field.to_owned(),
            clock: format!("{clock:?}"),
            epoch,
            time_ns: source_time_ns,
        });
    }
    let Some(mapped) = mapping.mapped_recorder_interval(source_time_ns) else {
        return invalid_clock_observation(
            field,
            "the source time does not map to the recorder clock",
        );
    };
    Ok(mapped)
}

fn invalid_clock_mapping(index: usize, reason: &'static str) -> Result<(), ValidationError> {
    Err(ValidationError::InvalidClockMapping { index, reason })
}

fn invalid_stage_stamp(field: &str, reason: &'static str) -> Result<(), ValidationError> {
    Err(ValidationError::InvalidStageStamp {
        field: field.to_owned(),
        reason,
    })
}

fn invalid_clock_observation<T>(field: &str, reason: &'static str) -> Result<T, ValidationError> {
    Err(ValidationError::InvalidClockObservation {
        field: field.to_owned(),
        reason,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
