//! A recording command sender, standing in for the Aviate uplink.
//!
//! The sender echoes each transmitted setpoint back as the flight
//! controller's effective setpoint on the next simulator sample, which is
//! what a controller that accepted the command does. Every test that needs
//! another behavior asks for it explicitly.

use flight_tune::Digest;
use flight_tune_aviate::direct_transport::{
    DirectCommandSender, DirectSenderError, DirectSenderIdentity, DirectSetpoint,
    EffectiveSetpointReport, TransmittedDirectCommand,
};

/// One simulator sample at the flight controller's 80 Hz setpoint rate.
pub const SAMPLE_PERIOD_NS: u64 = 12_500_000;

/// The command endpoint every frame is addressed to.
pub const ENDPOINT: &str = "127.0.0.1:20000";

/// A flight controller that answers what it was told, unless told not to.
pub struct RecordingSender {
    now_ns: u64,
    sequence: u8,
    commands: u32,
    effective: Option<EffectiveSetpointReport>,
    substitute_pitch: Option<f64>,
    reads: u32,
    silent_after_reads: Option<u32>,
}

impl RecordingSender {
    /// Creates one sender that echoes every command back.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            now_ns: 0,
            sequence: 0,
            commands: 0,
            effective: None,
            substitute_pitch: None,
            reads: 0,
            silent_after_reads: None,
        }
    }

    /// Reports a pitch the transport never asked for.
    #[must_use]
    pub const fn substituting_pitch(mut self, pitch_rad: f64) -> Self {
        self.substitute_pitch = Some(pitch_rad);
        self
    }

    /// Goes silent after the baseline block, so a step has no exact source.
    ///
    /// Freezing a baseline reads the raw source twice: once to choose the
    /// candidate baseline and once to read the command back. The source
    /// answers both and reports nothing after, which is the state a step
    /// meets when the raw source stops.
    #[must_use]
    pub const fn silent_after_baseline(mut self) -> Self {
        self.silent_after_reads = Some(2);
        self
    }
}

impl DirectCommandSender for RecordingSender {
    fn command_endpoint(&self) -> String {
        ENDPOINT.to_owned()
    }

    fn now_ns(&mut self) -> Result<u64, DirectSenderError> {
        Ok(self.now_ns)
    }

    fn transmit_exact_blocking(
        &mut self,
        setpoint: DirectSetpoint,
    ) -> Result<TransmittedDirectCommand, DirectSenderError> {
        self.sequence = self.sequence.wrapping_add(1);
        self.commands = self.commands.wrapping_add(1);
        let transmitted_at_ns = self.now_ns;
        let sample_time_ns = transmitted_at_ns.wrapping_add(SAMPLE_PERIOD_NS);
        self.now_ns = sample_time_ns;
        let reported = self
            .substitute_pitch
            .map_or(setpoint, |pitch_rad| DirectSetpoint {
                pitch_rad,
                ..setpoint
            });
        self.effective = Some(EffectiveSetpointReport {
            setpoint: reported,
            sample_sequence: sample_time_ns / SAMPLE_PERIOD_NS,
            sample_time_ns,
            estimate_time_ns: sample_time_ns.wrapping_sub(1_000_000),
            simulator_truth_time_ns: sample_time_ns,
        });
        Ok(TransmittedDirectCommand {
            setpoint,
            sender: DirectSenderIdentity {
                endpoint: ENDPOINT.to_owned(),
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
        self.reads = self.reads.wrapping_add(1);
        if self
            .silent_after_reads
            .is_some_and(|limit| self.reads > limit)
        {
            return Ok(None);
        }
        Ok(self.effective)
    }

    fn is_stable_blocking(&mut self) -> Result<bool, DirectSenderError> {
        Ok(true)
    }
}
