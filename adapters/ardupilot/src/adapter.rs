//! The ArduPilot `VehicleAdapter`: telemetry from the shared MAVLink
//! sampling, and GUIDED-mode flight control behind the per-scope
//! link-loss latch.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use pilotage_adapter_api::{
    ActionResult, AdapterCapabilities, ApplyOutcome, Disposition, FcCommandAck, LinkLossEnactError,
    LinkLossPolicy, RejectReason, SourceIncarnation, StepBudget, StepOutcome, TelemetryBatch,
    VehicleAdapter, VideoSource,
};
use pilotage_mavlink::ardupilot::ArduCopterMode;
use pilotage_mavlink::codec::MAV_CMD_COMPONENT_ARM_DISARM;
use pilotage_mavlink::sampling::{
    ArmTracker, OperationalPose, VehicleReports, battery_sample, estimate_batch, fc_state_sample,
    operational_pose, report_only_sample,
};
use pilotage_mavlink::{LinkState, MavlinkLink};
use pilotage_protocol::{ActionKind, ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;
use tracing::{info, warn};

use crate::config::ArduPilotConfig;
use crate::error::ArduPilotAdapterError;
use crate::uplink::ArduPilotUplink;

mod advertisement;
mod control;
mod link_loss;
#[cfg(test)]
mod tests;

/// The control scope: four canonical flight axes as velocity demands.
pub const FLIGHT_SCOPE: &str = "vehicle.motion";
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
/// Logical button whose press disarms (or, airborne, lands) the vehicle.
pub const DISARM_BUTTON: u16 = 1;

/// Telemetry plus GUIDED velocity control for ArduCopter. Real-time
/// (ADR-0013): the FC advances on its own clock; `step` reports the latest
/// observed source time as the simulation tick.
#[derive(Debug)]
pub struct ArduPilotAdapter {
    vehicle: VehicleId,
    // The shared receive-link cache; EKF_STATUS_REPORT authorizes it.
    state: Arc<Mutex<LinkState>>,
    _link: Option<MavlinkLink>,
    uplink: Option<ArduPilotUplink>,
    arm: ArmTracker,
    incarnation: SourceIncarnation,
    started_at: Instant,
    // The latest flight mode logged, so a change is logged once.
    reported_mode: Option<ArduCopterMode>,
    // The arm command whose refusal reasons were already logged.
    reported_refusal: Option<Instant>,
    // Per-scope link-loss latch (ADR-0008).
    link_loss_policy: BTreeMap<ScopeId, LinkLossPolicy>,
}

impl ArduPilotAdapter {
    /// Binds the configured MAVLink endpoint and wires the GUIDED command
    /// path onto the link's own socket.
    ///
    /// # Errors
    ///
    /// Returns [`ArduPilotAdapterError::Link`] when the link cannot bind.
    pub async fn start(
        vehicle: VehicleId,
        config: ArduPilotConfig,
    ) -> Result<Self, ArduPilotAdapterError> {
        let link_config = config.link_config();
        let mut link =
            MavlinkLink::start(link_config, SourceIncarnation::new(rand_incarnation())).await?;
        let uplink = link.take_setpoint_sender().map(|velocity| {
            ArduPilotUplink::new(
                link.command_sender(),
                velocity,
                link_config.system_id,
                link_config.component_id,
            )
        });
        info!(endpoint = %config.endpoint, "ArduPilot adapter started");
        Ok(Self::assemble(vehicle, link.state(), Some(link), uplink))
    }

    fn assemble(
        vehicle: VehicleId,
        state: Arc<Mutex<LinkState>>,
        link: Option<MavlinkLink>,
        uplink: Option<ArduPilotUplink>,
    ) -> Self {
        Self {
            vehicle,
            state,
            _link: link,
            uplink,
            arm: ArmTracker::default(),
            incarnation: SourceIncarnation::new(rand_incarnation()),
            started_at: Instant::now(),
            reported_mode: None,
            reported_refusal: None,
            link_loss_policy: BTreeMap::new(),
        }
    }

    /// Wires an adapter around a caller-supplied cache and uplink, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn from_parts(
        vehicle: VehicleId,
        state: Arc<Mutex<LinkState>>,
        uplink: ArduPilotUplink,
    ) -> Self {
        Self::assemble(vehicle, state, None, Some(uplink))
    }

    fn current_pose(&self) -> Option<OperationalPose> {
        operational_pose(&*self.state.lock().ok()?)
    }

    fn fc_armed(&self) -> bool {
        self.arm.report().is_some_and(|report| report.armed)
    }

    /// Folds the latest heartbeat, logs a flight-mode change, and logs the
    /// FC's reasons for a refused arm once per command.
    fn observe_fc(&mut self) {
        let Ok(latest) = self.state.lock() else {
            return;
        };
        self.arm.observe(&latest);
        let mode = latest
            .fc_reports
            .custom_mode
            .map(ArduCopterMode::from_custom_mode);
        if mode.is_some() && mode != self.reported_mode {
            info!(?mode, "ArduCopter flight mode");
            self.reported_mode = mode;
        }
        let Some((true, sent_at)) = self
            .uplink
            .as_ref()
            .and_then(ArduPilotUplink::last_arm_command)
        else {
            return;
        };
        let refused = latest
            .fc_reports
            .command_ack(MAV_CMD_COMPONENT_ARM_DISARM)
            .is_some_and(|ack| ack.received_at >= sent_at && ack.result != 0);
        if !refused || self.reported_refusal == Some(sent_at) {
            return;
        }
        self.reported_refusal = Some(sent_at);
        let reasons: Vec<&str> = latest
            .fc_reports
            .status_texts
            .iter()
            .filter(|text| text.received_at >= sent_at)
            .map(|text| text.text.as_str())
            .filter(|text| text.starts_with("PreArm") || text.starts_with("Arm"))
            .collect();
        warn!(?reasons, "ArduCopter refused to arm");
    }

    /// The FC's verdict on the latest arm or disarm this adapter sent.
    fn last_arm_ack(&self) -> Option<FcCommandAck> {
        let (arm, sent_at) = self.uplink.as_ref()?.last_arm_command()?;
        let ack = self
            .state
            .lock()
            .ok()?
            .fc_reports
            .command_ack(MAV_CMD_COMPONENT_ARM_DISARM)?;
        (ack.received_at >= sent_at).then_some(FcCommandAck {
            arm,
            result: u32::from(ack.result),
        })
    }
}

/// A random attachment identity; each adapter start is a distinct
/// source incarnation.
fn rand_incarnation() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    bytes[..8].copy_from_slice(&now.as_nanos().to_le_bytes()[..8]);
    bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    bytes
}

impl VehicleAdapter for ArduPilotAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        self.advertised_capabilities()
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let tick = self.step(StepBudget { ticks: 0 }).now;
        if self.link_loss_policy.contains_key(&frame.scope) {
            return ApplyOutcome::new(tick, Disposition::Rejected(RejectReason::LinkLossEngaged));
        }
        self.observe_fc();
        self.apply_flight(frame, tick)
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        self.observe_fc();
        if let Some(uplink) = self.uplink.as_mut() {
            uplink.observe_armed(self.arm.report().is_none_or(|report| report.armed));
        }
        let fc_state = fc_state_sample(
            self.arm.report(),
            self.last_arm_ack(),
            self.incarnation,
            self.started_at,
        );
        let battery = battery_sample(
            self.state.lock().ok().and_then(|latest| latest.battery),
            self.incarnation,
            self.started_at,
        );
        let mut batch = estimate_batch(self.vehicle, &self.state);
        let reports = VehicleReports {
            fc_state,
            gimbal: None,
            battery,
        };
        if batch.samples.is_empty() && reports.any() {
            let tick = self.step(StepBudget { ticks: 0 }).now;
            batch.samples.push(report_only_sample(self.vehicle, tick));
        }
        reports.attach(&mut batch);
        batch
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        vec![]
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
        let tick = self
            .state
            .lock()
            .ok()
            .and_then(|latest| latest.kinematics)
            .map_or(0, |kin| u64::from(kin.time_boot_ms).wrapping_mul(1_000_000));
        StepOutcome {
            advanced: 0,
            now: SimTick::new(tick),
        }
    }
}

/// Results for a frame's actions after the flight scope handled them.
fn action_results(frame: &ScopedControlFrame, handled: &[ActionKind]) -> Vec<ActionResult> {
    frame
        .actions
        .iter()
        .map(|action| {
            if handled.contains(&action.kind()) {
                ActionResult::accepted(*action)
            } else {
                ActionResult::rejected(*action, "not supported on the flight scope")
            }
        })
        .collect()
}
