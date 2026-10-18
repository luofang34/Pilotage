//! Flight-control gating (typed commands), reset handling, and the
//! velocity-envelope stick conversion — the same gate discipline as the
//! Aviate adapter: link-loss latch, reset/disarm actions, then the
//! commanded-reset latch.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{ActionResult, ApplyOutcome, Disposition, RejectReason};
use pilotage_protocol::{ActionKind, ControlIntent, ScopedControlFrame, VelocityIntent};
use pilotage_timing::SimTick;

use super::Px4Adapter;
use super::advertisement::FlightEnvelope;
use crate::uplink::direct::{DirectDemand, DirectDisposition, euler_of};
use crate::uplink::{
    MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS, StickFrameDisposition,
};

/// Reset clearance uses the same 5%-of-full-envelope scale as host recovery.
const RESET_CLEAR_DEADBAND: f32 = 0.05;

/// The engaged commanded-reset latch: the estimate stream's source epoch
/// observed at engagement. `engaged_epoch` is `None` when no estimate
/// stream was observable at that moment; the epoch-advance clearance can
/// then never be satisfied, which fails closed.
#[derive(Debug, Clone, Copy)]
pub(super) struct ResetLatch {
    engaged_epoch: Option<u32>,
}

/// Whether the frame carries an action of `kind`.
pub(super) fn has_action(frame: &ScopedControlFrame, kind: ActionKind) -> bool {
    frame.actions.iter().any(|action| action.kind() == kind)
}

pub(super) fn rejected_control(tick: SimTick, reason: RejectReason) -> ApplyOutcome {
    ApplyOutcome::new(tick, Disposition::Rejected(reason))
}

/// Per-action results for a frame whose disarm short-circuits it: the disarm
/// (and a sim reset that already spawned) report accepted; anything else the
/// frame carried is not executed this frame.
fn disarm_frame_action_result(
    action: pilotage_protocol::ControlAction,
) -> pilotage_adapter_api::ActionResult {
    match action.kind() {
        ActionKind::Disarm | ActionKind::SimReset => {
            pilotage_adapter_api::ActionResult::accepted(action)
        }
        _ => pilotage_adapter_api::ActionResult::rejected(
            action,
            "not executed: the frame's disarm short-circuits it",
        ),
    }
}

/// Converts a typed velocity (m/s, rad/s in the advertised envelope) back to
/// the normalized sticks the uplink's slew-limited setpoint path consumes:
/// the exact inverse of the envelope scaling a client applies, so a
/// full-envelope command flies exactly like full stick. Out-of-envelope
/// values clamp.
pub(super) fn sticks_from_velocity(velocity: &VelocityIntent) -> ([f32; 4], bool) {
    let normalize = |value: f32, limit: f32| {
        let normalized = value / limit;
        let clamped = if normalized.is_nan() {
            0.0
        } else {
            normalized.clamp(-1.0, 1.0)
        };
        (clamped, clamped != normalized)
    };
    let (roll, c0) = normalize(velocity.vy, MAX_HORIZONTAL_MPS);
    let (pitch, c1) = normalize(velocity.vx, MAX_HORIZONTAL_MPS);
    // + throttle stick = climb; body-FRD +z is down.
    let (throttle, c2) = normalize(-velocity.vz, MAX_VERTICAL_MPS);
    let (yaw, c3) = normalize(velocity.yaw_rate, MAX_YAW_RATE_RPS);
    ([roll, pitch, throttle, yaw], c0 || c1 || c2 || c3)
}

/// Reset clearance: every velocity component inside the envelope-scaled
/// deadband and no discrete action riding the frame. A frame without a
/// velocity intent demonstrates nothing.
fn frame_is_neutral(frame: &ScopedControlFrame, envelope: FlightEnvelope) -> bool {
    let Some(ControlIntent::Velocity(v)) = frame.intent else {
        return false;
    };
    frame.actions.is_empty()
        && v.vx.abs() <= envelope.max_linear * RESET_CLEAR_DEADBAND
        && v.vy.abs() <= envelope.max_linear * RESET_CLEAR_DEADBAND
        && v.vz.abs() <= envelope.max_vertical * RESET_CLEAR_DEADBAND
        && v.yaw_rate.abs() <= envelope.max_angular * RESET_CLEAR_DEADBAND
}

impl Px4Adapter {
    /// Runs the SITL reset script (debounced to one per 5 s), engaging
    /// the commanded-reset latch: the PX4 this adapter was talking to
    /// is about to restart, so every cached measurement loses its
    /// authority to validate control. `PILOTAGE_RESET_CMD` overrides
    /// the script path.
    pub(super) fn spawn_reset(&mut self) {
        let now = Instant::now();
        if self
            .last_reset
            .is_some_and(|last_reset| now.duration_since(last_reset) < Duration::from_secs(5))
        {
            return;
        }
        self.last_reset = Some(now);
        let engaged_epoch = self.observed_source_epoch();
        tracing::info!(
            ?engaged_epoch,
            "reset latch engaged; control suppressed until a fresh PX4 stream and neutral input"
        );
        self.reset_latch = Some(ResetLatch { engaged_epoch });
        #[cfg(test)]
        {
            self.reset_spawns = self.reset_spawns.wrapping_add(1);
        }
        #[cfg(not(test))]
        run_reset_command();
    }

    /// The pre-pose gate chain for one flight frame: the typed-only
    /// contract, structural checks, the link-loss latch, reset/disarm
    /// action handling, and the commanded-reset latch. `Some` is the early
    /// outcome; `None` lets the caller proceed to measurement-dependent
    /// control.
    pub(super) fn gated_flight_outcome(
        &mut self,
        frame: &ScopedControlFrame,
        tick: SimTick,
    ) -> Option<ApplyOutcome> {
        if self.uplink.is_none() && self.fixed_wing.is_none() {
            return Some(rejected_control(tick, RejectReason::UnknownScope));
        }
        if let Err(reason) = self.validate_flight_frame(frame) {
            return Some(rejected_control(tick, reason));
        }
        // Typed-only consumption: the session host translates any legacy
        // payload at its compatibility boundary before delivery.
        if frame.carries_payload() || !frame.carries_typed() {
            return Some(rejected_control(
                tick,
                RejectReason::Other("the flight scope consumes typed commands only".to_owned()),
            ));
        }
        // Disarm is handled before the commanded-reset latch, but only
        // after the link-loss gate above has admitted the frame.
        if has_action(frame, ActionKind::Disarm) {
            if let Some(fixed_wing) = self.fixed_wing.as_mut() {
                fixed_wing.send_disarm();
            } else if let Some(uplink) = self.uplink.as_mut() {
                uplink.send_disarm();
            } else {
                return Some(rejected_control(tick, RejectReason::UnknownScope));
            }
            return Some(ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results: frame
                    .actions
                    .iter()
                    .map(|action| disarm_frame_action_result(*action))
                    .collect(),
            });
        }
        // The commanded-reset latch: cached measurements inside the
        // freshness budget are pre-reset data, and the rebooting FC
        // may accept commands before its estimator converges.
        if self.reset_latch_blocks(frame) {
            return Some(rejected_control(tick, RejectReason::ResetInProgress));
        }
        None
    }

    /// Whether the commanded-reset latch suppresses this frame,
    /// attempting clearance first: the estimate stream must have
    /// entered a new source epoch, the frame must be neutral, and a
    /// full pose must be recoverable from the fresh stream.
    fn reset_latch_blocks(&mut self, frame: &ScopedControlFrame) -> bool {
        let Some(latch) = self.reset_latch else {
            return false;
        };
        let epoch_advanced = matches!(
            (latch.engaged_epoch, self.observed_source_epoch()),
            (Some(engaged), Some(current)) if current != engaged
        );
        if epoch_advanced
            && frame_is_neutral(frame, self.flight_envelope())
            && self.current_pose().is_some()
        {
            tracing::info!("reset latch cleared: fresh PX4 stream and neutral input");
            self.reset_latch = None;
            return false;
        }
        true
    }

    /// The estimate stream's current acquisition epoch, when observable.
    fn observed_source_epoch(&self) -> Option<u32> {
        let source = self.estimate.as_ref()?;
        let latest = source.state.lock().ok()?;
        Some(latest.source_epoch)
    }
}

/// Spawns the reset script without waiting for it. Not compiled for
/// tests: the script resets the live simulator and restarts PX4.
#[cfg(not(test))]
fn run_reset_command() {
    let script = std::env::var("PILOTAGE_RESET_CMD").unwrap_or_else(|_| {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .map_or_else(|| ".".to_owned(), |path| path.display().to_string())
            + "/scripts/reset-px4-sim.sh"
    });
    tracing::info!(%script, "simulation reset requested from the viewer");
    if let Err(error) = std::process::Command::new(&script).spawn() {
        tracing::warn!(%error, %script, "reset script failed to spawn");
    }
}

impl super::Px4Adapter {
    /// The simulator lifecycle scope: `SimReset` under its OWN lease
    /// (SIM-01) — flight authority neither grants nor implies it. Only a
    /// simulation-profile adapter advertises this scope; anything but a
    /// reset is refused defensively.
    pub(super) fn apply_sim_lifecycle(
        &mut self,
        frame: &pilotage_protocol::ScopedControlFrame,
        tick: pilotage_timing::SimTick,
    ) -> pilotage_adapter_api::ApplyOutcome {
        let mut action_results = Vec::with_capacity(frame.actions.len());
        for action in &frame.actions {
            match action.kind() {
                pilotage_protocol::ActionKind::SimReset => {
                    self.spawn_reset();
                    action_results.push(pilotage_adapter_api::ActionResult::accepted(*action));
                }
                _ => action_results.push(pilotage_adapter_api::ActionResult::rejected(
                    *action,
                    "only sim reset lives on the lifecycle scope",
                )),
            }
        }
        pilotage_adapter_api::ApplyOutcome {
            tick,
            disposition: pilotage_adapter_api::Disposition::Accepted,
            action_results,
        }
    }
}

impl Px4Adapter {
    /// The multicopter flight path once the gates have admitted a frame:
    /// arm starts the offboard sequence, and a velocity intent becomes a
    /// slew-limited offboard setpoint.
    pub(super) fn apply_multicopter_flight(
        &mut self,
        frame: &ScopedControlFrame,
        tick: SimTick,
    ) -> ApplyOutcome {
        let Some(pose) = self.current_pose() else {
            return rejected_control(tick, RejectReason::MeasurementUnavailable);
        };
        let (current_yaw, current_vel) = (pose.yaw_rad, pose.vel_ned_mps);
        let armed = self.arm.report().is_some_and(|report| report.armed);
        let Some(uplink) = self.uplink.as_mut() else {
            return rejected_control(tick, RejectReason::UnknownScope);
        };
        if frame.intent.is_some() {
            uplink.resume_after_neutralize(armed, current_yaw, current_vel);
        }
        let mut action_results = Vec::with_capacity(frame.actions.len());
        for action in &frame.actions {
            match action.kind() {
                ActionKind::Arm => {
                    uplink.begin_arm(current_yaw);
                    action_results.push(ActionResult::accepted(*action));
                }
                // Nothing else is advertised for this scope (sim reset
                // lives on the lifecycle scope), so the session rejects it
                // before delivery — defensive, not a reachable path.
                _ => {
                    action_results.push(ActionResult::rejected(
                        *action,
                        "not supported on the flight scope",
                    ));
                }
            }
        }
        // An actions-only frame (arm) carries no motion demand; the setpoint
        // stream continues from the next frame.
        let disposition = frame.intent.map_or(Disposition::Accepted, |intent| {
            multicopter_intent(uplink, frame, intent, current_yaw, current_vel)
        });
        ApplyOutcome {
            tick,
            disposition,
            action_results,
        }
    }
}

/// Enacts one multicopter intent on the uplink. Velocity rides the flight
/// scope and the direct families ride the direct scope; any other pairing
/// is refused rather than reinterpreted. Leaving direct flight seeds the
/// velocity stream from the measured state.
fn multicopter_intent(
    uplink: &mut crate::uplink::Px4Uplink,
    frame: &ScopedControlFrame,
    intent: ControlIntent,
    yaw_rad: f32,
    vel_ned_mps: Option<[f32; 3]>,
) -> Disposition {
    let scope = frame.scope.as_str();
    let demand = match intent {
        ControlIntent::Velocity(velocity) if scope == super::FLIGHT_SCOPE => {
            uplink.enter_velocity(yaw_rad, vel_ned_mps);
            let (sticks, constrained) = sticks_from_velocity(&velocity);
            return match uplink.send_stick_frame(sticks[0], sticks[1], sticks[2], sticks[3]) {
                StickFrameDisposition::UplinkIdle => {
                    Disposition::Rejected(RejectReason::UplinkIdle)
                }
                StickFrameDisposition::Queued if constrained => Disposition::Constrained,
                StickFrameDisposition::Queued => Disposition::Accepted,
            };
        }
        ControlIntent::BodyRate(rates) if scope == super::DIRECT_SCOPE => (
            DirectDemand::BodyRate([rates.roll_rate, rates.pitch_rate, rates.yaw_rate]),
            rates.thrust,
        ),
        ControlIntent::AttitudeThrust(attitude)
            if scope == super::DIRECT_SCOPE
                && attitude.frame == pilotage_protocol::ReferenceFrame::LocalNed =>
        {
            let q = [attitude.qw, attitude.qx, attitude.qy, attitude.qz];
            (DirectDemand::Attitude(euler_of(q)), attitude.thrust)
        }
        _ => {
            return Disposition::Rejected(RejectReason::Other(
                "intent family or frame not advertised on this scope".to_owned(),
            ));
        }
    };
    match uplink.send_direct(demand.0, demand.1) {
        DirectDisposition::Flown => Disposition::Accepted,
        DirectDisposition::Constrained => Disposition::Constrained,
        DirectDisposition::UplinkIdle => Disposition::Rejected(RejectReason::UplinkIdle),
        DirectDisposition::ThrustUnknown => {
            Disposition::Rejected(RejectReason::MeasurementUnavailable)
        }
    }
}
