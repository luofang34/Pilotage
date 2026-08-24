//! Sample times and clock discontinuities.

use serde::{Deserialize, Serialize};

use crate::{ClockDomain, MAX_CLOCK_MAPPINGS, RunIdentity, ValidationError, validation::unique};

use super::Observed;

/// One time in a named source clock epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockReading {
    /// The source clock epoch.
    pub epoch: u64,
    /// The time in nanoseconds.
    pub time_ns: u64,
}

/// Times for one sample in each available clock domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleTime {
    /// The mandatory recorder monotonic time.
    pub recorder_monotonic_ns: u64,
    /// The input device time.
    pub device: Observed<ClockReading>,
    /// The control client time.
    pub client: Observed<ClockReading>,
    /// The adapter time.
    pub adapter: Observed<ClockReading>,
    /// The flight controller time.
    pub flight_controller: Observed<ClockReading>,
    /// The simulator time.
    pub simulator: Observed<ClockReading>,
    /// Clock domains that had a discontinuity before this sample.
    pub clock_discontinuities: Vec<ClockDomain>,
}

impl SampleTime {
    /// Gets the available reading for one clock domain.
    #[must_use]
    pub fn reading(&self, domain: ClockDomain) -> Option<ClockReading> {
        match domain {
            ClockDomain::Recorder => Some(ClockReading {
                epoch: 0,
                time_ns: self.recorder_monotonic_ns,
            }),
            _ => self.source_reading(domain).copied(),
        }
    }

    pub(crate) fn source_reading(&self, domain: ClockDomain) -> Option<&ClockReading> {
        match domain {
            ClockDomain::Device => self.device.value(),
            ClockDomain::Client => self.client.value(),
            ClockDomain::Recorder => None,
            ClockDomain::Adapter => self.adapter.value(),
            ClockDomain::FlightController => self.flight_controller.value(),
            ClockDomain::Simulator => self.simulator.value(),
        }
    }

    /// Reports if a clock discontinuity occurs before this sample.
    #[must_use]
    pub fn has_discontinuity(&self, domain: ClockDomain) -> bool {
        self.clock_discontinuities.contains(&domain)
    }

    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::count(
            "sample.time.clock_discontinuities",
            self.clock_discontinuities.len(),
            MAX_CLOCK_MAPPINGS,
        )?;
        unique(
            "sample.time.clock_discontinuities",
            &self.clock_discontinuities,
        )?;
        if self.has_discontinuity(ClockDomain::Recorder) {
            return invalid_clock_observation(
                "sample.time.clock_discontinuities",
                "the recorder clock cannot have a discontinuity in one run",
            );
        }
        self.device
            .validate_with("sample.time.device", ClockReading::validate)?;
        self.client
            .validate_with("sample.time.client", ClockReading::validate)?;
        self.adapter
            .validate_with("sample.time.adapter", ClockReading::validate)?;
        self.flight_controller
            .validate_with("sample.time.flight_controller", ClockReading::validate)?;
        self.simulator
            .validate_with("sample.time.simulator", ClockReading::validate)
    }

    pub(crate) fn validate_for_run(&self, run: &RunIdentity) -> Result<(), ValidationError> {
        const DOMAINS: [ClockDomain; 5] = [
            ClockDomain::Device,
            ClockDomain::Client,
            ClockDomain::Adapter,
            ClockDomain::FlightController,
            ClockDomain::Simulator,
        ];
        for domain in DOMAINS {
            let Some(reading) = self.source_reading(domain) else {
                continue;
            };
            run.validate_sample_clock(
                clock_field(domain),
                domain,
                reading.epoch,
                reading.time_ns,
                self.recorder_monotonic_ns,
            )?;
        }
        Ok(())
    }
}

impl ClockReading {
    fn validate(&self, _field: &str) -> Result<(), ValidationError> {
        Ok(())
    }
}

fn clock_field(domain: ClockDomain) -> &'static str {
    match domain {
        ClockDomain::Device => "sample.time.device",
        ClockDomain::Client => "sample.time.client",
        ClockDomain::Recorder => "sample.time.recorder_monotonic_ns",
        ClockDomain::Adapter => "sample.time.adapter",
        ClockDomain::FlightController => "sample.time.flight_controller",
        ClockDomain::Simulator => "sample.time.simulator",
    }
}

fn invalid_clock_observation<T>(field: &str, reason: &'static str) -> Result<T, ValidationError> {
    Err(ValidationError::InvalidClockObservation {
        field: field.to_owned(),
        reason,
    })
}
