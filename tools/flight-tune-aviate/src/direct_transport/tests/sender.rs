//! A recording command sender, standing in for the Aviate uplink.
//!
//! The sender echoes each transmitted setpoint back as the flight
//! controller's effective setpoint on the next simulator sample, which is
//! what a controller that accepted the command does. Every test that needs
//! a different behavior asks for it explicitly.

use flight_tune::Digest;

use super::super::DirectCommandSender;
use super::super::{
    DirectSenderError, DirectSenderIdentity, DirectSetpoint, EffectiveSetpointReport,
    TransmittedDirectCommand,
};
use super::SAMPLE_PERIOD_NS;

pub(super) const ENDPOINT: &str = "127.0.0.1:20000";

pub(super) struct RecordingSender {
    endpoint: String,
    now_ns: u64,
    sequence: u8,
    transmitted: Vec<DirectSetpoint>,
    effective: Option<EffectiveSetpointReport>,
    unstable_commands: u32,
    /// Reports this setpoint instead of the transmitted one, so a test can
    /// stage a flight controller that constrained the command.
    substitute_effective: Option<DirectSetpoint>,
    /// Reports no raw source at all.
    silent_source: bool,
    /// Keeps the reported sample where it is, however far the clock runs.
    hold_sample: bool,
}

impl RecordingSender {
    pub(super) fn new() -> Self {
        Self {
            endpoint: ENDPOINT.to_owned(),
            now_ns: 0,
            sequence: 0,
            transmitted: Vec::new(),
            effective: None,
            unstable_commands: 0,
            substitute_effective: None,
            silent_source: false,
            hold_sample: false,
        }
    }

    pub(super) fn with_endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = endpoint.to_owned();
        self
    }

    /// Reports the vehicle as unsettled for the first `commands` commands.
    pub(super) const fn unstable_for(mut self, commands: u32) -> Self {
        self.unstable_commands = commands;
        self
    }

    /// Reports a flight-controller setpoint that is not the transmitted one.
    pub(super) const fn substituting(mut self, effective: DirectSetpoint) -> Self {
        self.substitute_effective = Some(effective);
        self
    }

    /// Reports no raw direct source at all.
    pub(super) const fn silent(mut self) -> Self {
        self.silent_source = true;
        self
    }

    /// Seeds an already-active effective setpoint before the first command.
    pub(super) const fn reporting(mut self, report: EffectiveSetpointReport) -> Self {
        self.effective = Some(report);
        self
    }

    /// Keeps the reported sample where it is, so the clock can run past the
    /// causal bound without the source ever catching up.
    pub(super) const fn holding_sample(mut self) -> Self {
        self.hold_sample = true;
        self
    }

    pub(super) fn transmitted(&self) -> &[DirectSetpoint] {
        &self.transmitted
    }

    pub(super) fn clear_transmitted(&mut self) {
        self.transmitted.clear();
    }

    /// Stops the reported sample from advancing past the one it holds now.
    pub(super) const fn hold_sample_from_now(&mut self) {
        self.hold_sample = true;
    }

    /// Runs the clock forward without a command.
    pub(super) fn advance(&mut self, samples: u64) {
        self.now_ns = self
            .now_ns
            .wrapping_add(samples.wrapping_mul(SAMPLE_PERIOD_NS));
    }
}

impl DirectCommandSender for RecordingSender {
    fn command_endpoint(&self) -> String {
        self.endpoint.clone()
    }

    fn now_ns(&mut self) -> Result<u64, DirectSenderError> {
        Ok(self.now_ns)
    }

    fn transmit_exact_blocking(
        &mut self,
        setpoint: DirectSetpoint,
    ) -> Result<TransmittedDirectCommand, DirectSenderError> {
        self.transmitted.push(setpoint);
        self.sequence = self.sequence.wrapping_add(1);
        let transmitted_at_ns = self.now_ns;
        let sample_time_ns = transmitted_at_ns.wrapping_add(SAMPLE_PERIOD_NS);
        self.now_ns = sample_time_ns;
        if !self.hold_sample {
            self.effective = Some(EffectiveSetpointReport {
                setpoint: self.substitute_effective.unwrap_or(setpoint),
                sample_sequence: sample_time_ns / SAMPLE_PERIOD_NS,
                sample_time_ns,
                estimate_time_ns: sample_time_ns.wrapping_sub(1_000_000),
                simulator_truth_time_ns: sample_time_ns,
            });
        }
        Ok(TransmittedDirectCommand {
            setpoint,
            sender: DirectSenderIdentity {
                endpoint: self.endpoint.clone(),
                system_id: 1,
                component_id: 1,
                sequence: self.sequence,
                time_boot_ms: u32::try_from(transmitted_at_ns / 1_000_000).unwrap_or(u32::MAX),
                frame_digest: Digest::from_bytes([0xab; 32]),
            },
            transmitted_at_ns,
        })
    }

    fn effective_setpoint_blocking(
        &mut self,
    ) -> Result<Option<EffectiveSetpointReport>, DirectSenderError> {
        if self.silent_source {
            return Ok(None);
        }
        Ok(self.effective)
    }

    fn is_stable_blocking(&mut self) -> Result<bool, DirectSenderError> {
        if self.unstable_commands == 0 {
            return Ok(true);
        }
        self.unstable_commands = self.unstable_commands.wrapping_sub(1);
        Ok(false)
    }
}
