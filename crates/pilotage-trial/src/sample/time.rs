//! Sample times and clock discontinuities.

use serde::{Deserialize, Serialize};

use crate::{ClockDomain, MAX_CLOCK_MAPPINGS, ValidationError, validation::unique};

use super::Observed;

/// Times for one sample in each available clock domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleTime {
    /// The mandatory recorder monotonic time.
    pub recorder_monotonic_ns: u64,
    /// The input device time.
    pub device_ns: Observed<u64>,
    /// The control client time.
    pub client_ns: Observed<u64>,
    /// The adapter time.
    pub adapter_ns: Observed<u64>,
    /// The flight controller time.
    pub flight_controller_ns: Observed<u64>,
    /// The simulator time.
    pub simulator_ns: Observed<u64>,
    /// Clock domains that had a discontinuity before this sample.
    pub clock_discontinuities: Vec<ClockDomain>,
}

impl SampleTime {
    /// Gets the available time for one clock domain.
    #[must_use]
    pub fn get(&self, domain: ClockDomain) -> Option<u64> {
        match domain {
            ClockDomain::Recorder => Some(self.recorder_monotonic_ns),
            ClockDomain::Device => self.device_ns.value().copied(),
            ClockDomain::Client => self.client_ns.value().copied(),
            ClockDomain::Adapter => self.adapter_ns.value().copied(),
            ClockDomain::FlightController => self.flight_controller_ns.value().copied(),
            ClockDomain::Simulator => self.simulator_ns.value().copied(),
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
        self.device_ns
            .validate_with("sample.time.device_ns", present_time)?;
        self.client_ns
            .validate_with("sample.time.client_ns", present_time)?;
        self.adapter_ns
            .validate_with("sample.time.adapter_ns", present_time)?;
        self.flight_controller_ns
            .validate_with("sample.time.flight_controller_ns", present_time)?;
        self.simulator_ns
            .validate_with("sample.time.simulator_ns", present_time)
    }
}

fn present_time(_value: &u64, _field: &str) -> Result<(), ValidationError> {
    Ok(())
}
