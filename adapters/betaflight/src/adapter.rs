//! The Betaflight `VehicleAdapter`: typed control frames in, telemetry
//! samples out, with the link task doing the flying.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, LinkLossEnactError, LinkLossPolicy, StepBudget, StepOutcome,
    TelemetryBatch, VehicleAdapter, VideoSource,
};
use pilotage_protocol::{ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;
use tokio::sync::watch;

use crate::config::{BetaflightConfig, MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS};
use crate::link::{self, LinkCommand, LinkReport, Observation};
use crate::outer_loop::Envelope;

mod advertisement;
mod control;
mod direct_intent;
mod link_loss;
mod sampling;
#[cfg(test)]
mod tests;

/// The assisted control scope: four canonical flight axes as velocity
/// demands.
pub const FLIGHT_SCOPE: &str = pilotage_protocol::MOTION_SCOPE;
/// The direct control scope: body rates or an attitude with thrust, in
/// the same authority group as [`FLIGHT_SCOPE`], so one client flies one
/// of them at a time.
pub const DIRECT_SCOPE: &str = pilotage_protocol::DIRECT_SCOPE;
/// Canonical `roll` axis (0): lateral velocity, + = right.
pub const ROLL_AXIS: u16 = 0;
/// Canonical `pitch` axis (1): forward velocity, + = forward.
pub const PITCH_AXIS: u16 = 1;
/// Canonical `throttle` axis (2): climb rate, + = climb.
pub const THROTTLE_AXIS: u16 = 2;
/// Canonical `yaw` axis (3): yaw rate, + = clockwise.
pub const YAW_AXIS: u16 = 3;
/// Logical button whose press arms the vehicle.
pub const ARM_BUTTON: u16 = 0;
/// Logical button whose press disarms the vehicle.
pub const DISARM_BUTTON: u16 = 1;

/// A poll older than this is withheld from telemetry and from control,
/// so a silent flight controller ages into staleness instead of replaying.
const WITHHOLD_AFTER: Duration = Duration::from_secs(3);

/// The envelope a full-scale demand maps to.
pub(crate) const ENVELOPE: Envelope = Envelope {
    horizontal: MAX_HORIZONTAL_MPS as f64,
    vertical: MAX_VERTICAL_MPS as f64,
    yaw_rate: MAX_YAW_RATE_RPS as f64,
};

/// The Betaflight adapter. Real-time (ADR-0013): Betaflight runs on its
/// own clock, and `step` reports the latest poll's host time.
#[derive(Debug)]
pub struct BetaflightAdapter {
    vehicle: VehicleId,
    commands: watch::Sender<LinkCommand>,
    reports: watch::Receiver<LinkReport>,
    link: Option<tokio::task::JoinHandle<()>>,
    incarnation: pilotage_adapter_api::SourceIncarnation,
    started_at: Instant,
    // Per-scope link-loss latch (ADR-0008).
    link_loss_policy: BTreeMap<ScopeId, LinkLossPolicy>,
}

impl BetaflightAdapter {
    /// Starts the MSP link task on the current Tokio runtime. The link
    /// connects (and reconnects) on its own, so the adapter starts before
    /// the flight controller is reachable and reports no telemetry until
    /// it is.
    ///
    /// Call it inside a Tokio runtime, as the session host does for every
    /// adapter: the link is a spawned task.
    #[must_use]
    pub fn start(vehicle: VehicleId, config: BetaflightConfig) -> Self {
        let (commands, command_rx) = watch::channel(LinkCommand::default());
        let (report_tx, reports) = watch::channel(LinkReport::default());
        let task = tokio::spawn(link::run(
            config.msp_endpoint,
            config.profile(),
            ENVELOPE,
            command_rx,
            report_tx,
        ));
        tracing::info!(endpoint = %config.msp_endpoint, profile = ?config.profile(), "Betaflight adapter started");
        Self {
            vehicle,
            commands,
            reports,
            link: Some(task),
            incarnation: pilotage_adapter_api::SourceIncarnation::new(attachment_identity()),
            started_at: Instant::now(),
            link_loss_policy: BTreeMap::new(),
        }
    }

    /// An adapter fed by a test-owned report channel, for tests.
    #[cfg(test)]
    pub(crate) fn detached(
        vehicle: VehicleId,
    ) -> (
        Self,
        watch::Sender<LinkReport>,
        watch::Receiver<LinkCommand>,
    ) {
        let (commands, command_rx) = watch::channel(LinkCommand::default());
        let (report_tx, reports) = watch::channel(LinkReport::default());
        let adapter = Self {
            vehicle,
            commands,
            reports,
            link: None,
            incarnation: pilotage_adapter_api::SourceIncarnation::new([0x42; 16]),
            started_at: Instant::now(),
            link_loss_policy: BTreeMap::new(),
        };
        (adapter, report_tx, command_rx)
    }

    /// The latest poll, unless it is older than the withholding budget.
    fn fresh_observation(&self) -> Option<Observation> {
        self.reports
            .borrow()
            .latest
            .filter(|observed| observed.snapshot.received_at.elapsed() <= WITHHOLD_AFTER)
    }

    fn tick_of(&self, at: Instant) -> SimTick {
        let since = at.saturating_duration_since(self.started_at);
        SimTick::new(u64::try_from(since.as_nanos()).unwrap_or(u64::MAX))
    }
}

impl Drop for BetaflightAdapter {
    fn drop(&mut self) {
        // The task owns the stream; ending it stops the RC frames, which
        // hands the vehicle to Betaflight's receiver-loss failsafe.
        if let Some(task) = self.link.take() {
            task.abort();
        }
    }
}

/// A per-start attachment identity from the clock and process id.
fn attachment_identity() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    bytes[..8].copy_from_slice(&now.as_nanos().to_le_bytes()[..8]);
    bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    bytes[12..].copy_from_slice(b"BTFL");
    bytes
}

impl VehicleAdapter for BetaflightAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        self.advertised_capabilities()
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        self.apply_flight_frame(frame)
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        self.telemetry_batch()
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        Vec::new()
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        self.enact_link_loss_policy(vehicle, scope, policy)
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        let now = self
            .fresh_observation()
            .map_or(SimTick::new(0), |observed| {
                self.tick_of(observed.snapshot.received_at)
            });
        StepOutcome { advanced: 0, now }
    }
}
