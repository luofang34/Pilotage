//! `VehicleAdapter` implementation over the Aviate vehicle's role-bound
//! links (ADR-0019, LINK-04): the MAVLink link carries the FC
//! operational estimate, the co-located shm block carries simulation
//! truth, and the uplink socket carries FC-owned state reports.

use std::collections::BTreeMap;
use std::time::Duration;

use pilotage_adapter_api::{
    ActionResult, AdapterCapabilities, ApplyOutcome, ControlFeelDescriptor, LinkLossPolicy,
    RejectReason, SourceIncarnation, StepBudget, StepOutcome, TelemetryBatch, VehicleAdapter,
    VideoSource,
};
use pilotage_control_feel::{FeelDigest, FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_protocol::{ControlAction, LogicalAxisId, ScopeId, ScopedControlFrame, VehicleId};

#[cfg(test)]
use std::sync::{Arc, Mutex};

#[cfg(test)]
use pilotage_mavlink::link::LinkState;

use crate::error::AviateAdapterError;
use crate::uplink::FlightUplink;

mod advertisement;
#[cfg(feature = "sim")]
mod camera;
mod profile;
mod sim_attachments;
pub use profile::AviateProfile;
#[cfg(not(feature = "sim"))]
use sim_attachments::no_camera;
use sim_attachments::{CameraBridge, Pointing, TruthOracle};

mod control;
mod control_feel;
mod flight;
#[cfg(feature = "sim")]
mod pointing;
mod sampling;
#[cfg(feature = "sim")]
mod shm_sampling;
mod sources;
mod startup;
use control::rejected_control;
use sampling::mavlink_batch;
use sources::{ArmReport, EstimateSource};
pub(crate) use startup::validate_aviate_profile_bindings;

/// The control scope exposes four canonical flight axes as DJI-style
/// velocity demands.
pub const FLIGHT_SCOPE: &str = "vehicle.motion";
/// The direct-flight scope (CTRL-01): attitude + collective thrust under its
/// OWN lease and authority generation — never a reinterpretation of the
/// velocity scope's numbers.
pub const DIRECT_SCOPE: &str = "vehicle.motion.direct";
/// The gimbal pointing scope (GIM-01, ADR-0006 vocabulary): pitch/yaw
/// line-of-sight rate demands, leased and fenced independently of flight.
/// Here the payload is a producer-rendered view, not a servo gimbal — the
/// adapter integrates the commanded rate into the pointing angle the
/// producer consumes.
#[cfg(feature = "sim")]
pub const GIMBAL_SCOPE: &str = "vehicle.gimbal";
/// Gimbal-scope button whose press recenters the payload view.
#[cfg(feature = "sim")]
pub const GIMBAL_NEUTRAL_BUTTON: u16 = 0;
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

/// Data older than this is withheld from telemetry entirely, so
/// downstream freshness models see the group's age grow instead of a
/// frozen value replaying forever (the same withholding discipline as
/// the Gazebo adapter's dead-reader path).
const WITHHOLD_AFTER: Duration = Duration::from_secs(3);

/// Telemetry-only adapter for the Aviate flight controller (ADR-0018).
///
/// Real-time (ADR-0013): the FC/simulation advances on its own clock;
/// `step` reports the latest observed vehicle time as the simulation
/// tick.
#[derive(Debug)]
pub struct AviateAdapter {
    vehicle: VehicleId,
    // The session profile this adapter was constructed for (LINK-04):
    // lifecycle capability is STRUCTURAL — a physical/RF profile neither
    // advertises nor executes simulator lifecycle commands.
    profile: AviateProfile,
    // Source roles are structural (LINK-04): the MAVLink link only ever
    // produces the FC operational estimate and the shm link only ever
    // produces the simulation-truth oracle. Neither substitutes for the
    // other: a missing estimate rejects state-dependent control instead
    // of borrowing truth.
    estimate: Option<EstimateSource>,
    truth: Option<Box<TruthOracle>>,
    uplink: Option<FlightUplink>,
    control_feel_profiles: Option<control_feel::ControlFeelProfiles>,
    control_feel_changed: bool,
    // Pilotage's Gazebo sidecar bridges the flight world's camera topics;
    // the adapter remains usable without video when the sidecar cannot spawn.
    frames: Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>>,
    // The producer link. Held (not underscore-parked) because the gimbal
    // scope enacts through it: pointing and zoom are commands to the
    // producer that renders the payload view.
    camera_bridge: Option<CameraBridge>,
    // How many consecutive truth samples have been published with no fix
    // joined to them. A sensor that never speaks, a topic nobody
    // publishes, and two clocks that do not share an origin all look the
    // same from here: no position, forever, in silence. The count is what
    // turns that silence into one report.
    #[cfg_attr(not(feature = "sim"), allow(dead_code))]
    navsat_join_failures: u64,
    // The commanded pointing of the payload view, integrated from the
    // scope's rate demands. `None` when no producer accepts commands,
    // and structurally uninhabited in a flight build.
    #[cfg_attr(not(feature = "sim"), allow(dead_code))]
    pointing: Option<Pointing>,
    _frame_forwarder: Option<tokio::task::JoinHandle<()>>,
    // Latest FC arm report from uplink heartbeats, with its receive
    // metadata; `None` until the FC has reported at least once.
    arm: Option<ArmReport>,
    // Identity under which arm reports are stamped.
    arm_incarnation: SourceIncarnation,
    // Zero point of the host-monotonic acquisition clock.
    started_at: std::time::Instant,
    last_reset: Option<std::time::Instant>,
    /// Whether the last payload-view publish failed, so a dead
    /// producer link logs one transition line instead of storming the
    /// telemetry tick.
    view_publish_failed: bool,
    // Per-scope link-loss latch (ADR-0008): a gimbal-scope policy must not
    // suppress or neutralize motion, so the latch is keyed by scope.
    link_loss_policy: BTreeMap<ScopeId, LinkLossPolicy>,
    // Commanded-reset latch: engaged when a sim reset is requested,
    // cleared only by a fresh estimate source epoch plus demonstrated
    // neutral input (control::ResetLatch). While engaged, everything
    // except disarm is rejected.
    reset_latch: Option<control::ResetLatch>,
    // Reset script spawns recorded instead of executed, so tests can
    // press the reset button without running the real script (which
    // kills any live SITL FC on the machine).
    #[cfg(test)]
    reset_spawns: u32,
}

impl AviateAdapter {
    /// Stages one validated control-feel artifact.
    ///
    /// The active artifact does not change until a flight frame is neutral
    /// under both artifacts.
    ///
    /// # Errors
    ///
    /// Returns an error if the artifact does not match this adapter.
    pub fn stage_control_feel(
        &mut self,
        candidate: ValidatedFlightFeelProfile,
    ) -> Result<FeelDigest, AviateAdapterError> {
        self.stage_validated_control_feel(candidate)
    }

    /// Stages one named operator feel mode.
    ///
    /// An operator chooses between Precision, Balanced and Agile — three laws
    /// that differ in degree and never in kind. This is that choice, by name,
    /// rather than an arbitrary profile: a caller that could supply any
    /// profile could supply one nobody qualified, and a mode a vehicle
    /// advertises is a mode somebody stood behind.
    ///
    /// The mode arrives at the same neutral boundary a rollback uses, so the
    /// law does not change under a deflected stick.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when the vehicle is physical, when the
    /// profile is not one this vehicle accepts, or when no control-feel
    /// artifact is active.
    pub fn request_feel_mode(&mut self, mode: FeelMode) -> Result<FeelDigest, AviateAdapterError> {
        let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped(mode))
            .map_err(|source| AviateAdapterError::InvalidControlFeel { source })?;
        self.stage_control_feel(profile)
    }

    fn stage_validated_control_feel(
        &mut self,
        candidate: ValidatedFlightFeelProfile,
    ) -> Result<FeelDigest, AviateAdapterError> {
        if self.profile == AviateProfile::Physical {
            return Err(AviateAdapterError::PhysicalControlFeelOverride {
                profile_id: candidate.profile().profile_id.clone(),
            });
        }
        startup::validate_adapter_control_feel(&candidate)?;
        self.control_feel_profiles
            .as_mut()
            .ok_or_else(|| AviateAdapterError::UnsupportedControlFeel {
                detail: "the adapter has no active control-feel artifact".to_owned(),
            })?
            .stage(candidate)
    }

    /// Stages the complete prior artifact for rollback.
    ///
    /// The rollback uses the same neutral boundary as a forward activation.
    #[must_use]
    pub fn stage_control_feel_rollback(&mut self) -> bool {
        if self.profile == AviateProfile::Physical {
            return false;
        }
        self.control_feel_profiles
            .as_mut()
            .is_some_and(control_feel::ControlFeelProfiles::stage_rollback)
    }

    fn activate_pending_control_feel(
        &mut self,
        frame: &ScopedControlFrame,
        current_yaw_rad: f32,
    ) -> Result<bool, &'static str> {
        let (Some(profiles), Some(uplink)) = (&mut self.control_feel_profiles, &mut self.uplink)
        else {
            return Ok(false);
        };
        if !profiles.pending_is_neutral(frame, uplink.monotonic_now()) {
            return Ok(false);
        }
        match uplink.prepare_control_feel_activation(current_yaw_rad) {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(detail) => return Err(detail),
        }
        let Some(next) = profiles.commit_pending() else {
            return Err("the pending control-feel artifact was not available");
        };
        uplink.install_profile(next.profile);
        self.control_feel_changed = true;
        tracing::info!(
            feel_profile_id = %next.identity.profile_id,
            feel_schema = next.identity.schema,
            feel_digest = %next.identity.digest,
            "Aviate control-feel profile activated"
        );
        Ok(true)
    }

    /// Takes the raw-frame receiver for the host media task, if cameras
    /// are up and it has not been taken.
    pub fn subscribe_frames(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>> {
        self.frames.take()
    }

    /// Wires an adapter around a caller-supplied state cache, for tests.
    #[cfg(test)]
    pub(crate) fn from_state(vehicle: VehicleId, state: Arc<Mutex<LinkState>>) -> Self {
        Self {
            vehicle,
            profile: AviateProfile::Simulation,
            estimate: Some(EstimateSource { state, _link: None }),
            truth: None,
            uplink: None,
            control_feel_profiles: None,
            control_feel_changed: false,
            frames: None,
            camera_bridge: None,
            navsat_join_failures: 0,
            pointing: None,
            _frame_forwarder: None,
            arm: None,
            arm_incarnation: SourceIncarnation::new([0; 16]),
            started_at: std::time::Instant::now(),
            last_reset: None,
            view_publish_failed: false,
            reset_latch: None,
            reset_spawns: 0,
            link_loss_policy: BTreeMap::new(),
        }
    }

    /// Expires the bound uplink's post-arm quiet window, so tests step
    /// past it deterministically instead of sleeping wall-clock time.
    #[cfg(test)]
    pub(crate) fn expire_uplink_quiet_for_test(&mut self) {
        if let Some(uplink) = self.uplink.as_mut() {
            uplink.expire_quiet_for_test();
        }
    }

    /// Whether the bound uplink currently holds a captured position-hold
    /// point, for tests of the link-loss hold-invalidation contract.
    #[cfg(test)]
    pub(crate) fn uplink_hold_captured(&self) -> bool {
        self.uplink
            .as_ref()
            .is_some_and(crate::uplink::FlightUplink::hold_captured)
    }

    /// Overrides the constructed profile, for tests exercising the
    /// physical/RF shape.
    #[cfg(test)]
    pub(crate) fn with_profile(mut self, profile: AviateProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Installs a test uplink, for tests.
    #[cfg(test)]
    pub(crate) fn with_uplink(mut self, uplink: FlightUplink) -> Self {
        self.control_feel_profiles =
            control_feel::ControlFeelProfiles::new(uplink.active_profile_for_test().clone()).ok();
        self.uplink = Some(uplink);
        self
    }

    fn feel_entry(&self) -> Option<&control_feel::ControlFeelEntry> {
        self.control_feel_profiles
            .as_ref()
            .map(control_feel::ControlFeelProfiles::active)
    }

    /// The bound uplink, for tests that drive its manual clock.
    #[cfg(test)]
    pub(crate) fn uplink_mut(&mut self) -> Option<&mut FlightUplink> {
        self.uplink.as_mut()
    }

    fn validate_flight_frame(&self, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if frame.scope.as_str() != FLIGHT_SCOPE && frame.scope.as_str() != DIRECT_SCOPE {
            return Err(RejectReason::UnknownScope);
        }
        let known = [
            LogicalAxisId::new(ROLL_AXIS),
            LogicalAxisId::new(PITCH_AXIS),
            LogicalAxisId::new(THROTTLE_AXIS),
            LogicalAxisId::new(YAW_AXIS),
        ];
        for (axis, _) in &frame.payload.axes {
            if !known.contains(axis) {
                return Err(RejectReason::UnknownAxis);
            }
        }
        Ok(())
    }
}

/// Disposes each typed flight action: arm fires through the caller's hook,
/// the gate-honored sim reset acks, and anything unsupported here reports
/// rejected (the session gates unadvertised actions before delivery —
/// defensive, not a reachable path). Mode requests are unsupported: direct
/// flight is its OWN scope with its own lease, never a mode flip that
/// reinterprets this scope's numbers.
fn process_flight_actions(
    actions: &[ControlAction],
    mut send_arm: impl FnMut(f32),
    current_yaw: f32,
) -> Vec<ActionResult> {
    let mut action_results = Vec::with_capacity(actions.len());
    for action in actions {
        match *action {
            ControlAction::Arm => {
                send_arm(current_yaw);
                action_results.push(ActionResult::accepted(*action));
            }
            ControlAction::ModeRequest { .. } => {
                action_results.push(ActionResult::rejected(
                    *action,
                    "no mode requests: direct flight is the vehicle.motion.direct scope",
                ));
            }
            // Handled by the adapter before this point, which is where the
            // control-feel artifact lives. Reaching here means it was not, and
            // a request that quietly did nothing would leave an operator
            // believing the law had changed.
            ControlAction::FeelModeRequest { .. } => {
                action_results.push(ActionResult::rejected(
                    *action,
                    "the control-feel artifact was not reachable for this request",
                ));
            }
            ControlAction::SimReset
            | ControlAction::Disarm
            | ControlAction::GimbalRecenter
            | ControlAction::CameraZoomIn
            | ControlAction::CameraZoomOut => {
                action_results.push(ActionResult::rejected(
                    *action,
                    "not supported on the flight scope",
                ));
            }
        }
    }
    action_results
}

impl VehicleAdapter for AviateAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        self.advertised_capabilities()
    }

    fn take_control_feel_change(&mut self) -> Option<ControlFeelDescriptor> {
        if !core::mem::take(&mut self.control_feel_changed) {
            return None;
        }
        self.feel_entry()
            .map(advertisement::control_feel_descriptor)
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let tick = self.step(StepBudget { ticks: 0 }).now;
        if frame.scope.as_str() == pilotage_adapter_api::SIM_LIFECYCLE_SCOPE {
            return self.apply_sim_lifecycle(frame, tick);
        }
        // Per-scope link-loss latch (ADR-0010): a frame is suppressed
        // only while ITS scope has a policy engaged, so a gimbal failsafe
        // never suppresses motion and the reverse.
        if self.link_loss_policy.contains_key(&frame.scope) {
            return rejected_control(tick, RejectReason::LinkLossEngaged);
        }
        #[cfg(feature = "sim")]
        if frame.scope.as_str() == GIMBAL_SCOPE {
            // Pointing sits outside the flight gate chain: aiming a
            // payload view is not a flight demand and must not wait on
            // an estimate or an uplink.
            return self.apply_gimbal(frame, tick);
        }
        self.apply_flight(frame, tick)
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        self.advertised_video_sources()
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), pilotage_adapter_api::LinkLossEnactError> {
        self.enact_link_loss(vehicle, scope, policy)
    }

    fn step(&mut self, budget: StepBudget) -> StepOutcome {
        self.advance(budget)
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        self.collect_telemetry()
    }
}

#[cfg(test)]
mod tests;
