//! Flight-control gating (typed commands), reset handling, and the
//! velocity-envelope stick conversion.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{ApplyOutcome, Disposition, RejectReason};
use pilotage_protocol::{ActionKind, ControlIntent, ScopedControlFrame, VelocityIntent};
use pilotage_timing::SimTick;

use super::AviateAdapter;
use crate::uplink::{MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS};

/// Reset clearance uses the same 5%-of-full-envelope scale as host recovery.
const RESET_CLEAR_DEADBAND: f32 = 0.05;

/// The engaged commanded-reset latch: the estimate stream's source epoch
/// observed at engagement. `engaged_epoch` is `None` when no estimate
/// stream was observable at that moment; the epoch-advance clearance can
/// then never be satisfied, which fails closed (a profile without an
/// estimate stream cannot arm anyway).
#[derive(Debug, Clone, Copy)]
pub(super) struct ResetLatch {
    engaged_epoch: Option<u32>,
}

/// Reset clearance: every velocity component inside the envelope-scaled
/// deadband and no discrete action riding the frame. A frame without a
/// velocity intent demonstrates nothing.
fn frame_is_neutral(frame: &ScopedControlFrame) -> bool {
    let Some(ControlIntent::Velocity(v)) = frame.intent else {
        return false;
    };
    frame.actions.is_empty()
        && v.vx.abs() <= MAX_HORIZONTAL_MPS * RESET_CLEAR_DEADBAND
        && v.vy.abs() <= MAX_HORIZONTAL_MPS * RESET_CLEAR_DEADBAND
        && v.vz.abs() <= MAX_VERTICAL_MPS * RESET_CLEAR_DEADBAND
        && v.yaw_rate.abs() <= MAX_YAW_RATE_RPS * RESET_CLEAR_DEADBAND
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
fn action_result_for_disarm_frame(
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
/// the normalized sticks the uplink's setpoint shaping consumes: the exact
/// inverse of the envelope scaling a client applies, so a full-envelope
/// command flies exactly like full stick. Out-of-envelope values clamp.
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

impl AviateAdapter {
    /// Runs the SITL reset script (debounced to one per 5 s): world
    /// reset + FC restart, fire-and-forget. `PILOTAGE_RESET_CMD`
    /// overrides the script path. Engages the commanded-reset latch:
    /// the FC this adapter was talking to is about to die, so every
    /// cached measurement loses its authority to validate control.
    pub(super) fn spawn_reset(&mut self) {
        let now = Instant::now();
        if self
            .last_reset
            .is_some_and(|last_reset| now.duration_since(last_reset) < Duration::from_secs(5))
        {
            return;
        }
        self.last_reset = Some(now);
        // The restarted FC re-reports arm state under fresh heartbeats;
        // the pre-reset report must not survive as current.
        self.arm = None;
        let engaged_epoch = self.observed_source_epoch();
        tracing::info!(
            ?engaged_epoch,
            "reset latch engaged; control suppressed until a fresh FC stream and neutral input"
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
        if self.uplink.is_none() {
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
        // The link-loss latch: while a policy is engaged the FC holds its
        // policy state (braked hover) and ordinary frames are suppressed,
        // so a newly granted holder with deflected sticks cannot fly the
        // vehicle out of it. The host clears the latch only after the
        // holder demonstrates neutral input.
        if self.link_loss_policy.contains_key(&frame.scope) {
            return Some(rejected_control(tick, RejectReason::LinkLossEngaged));
        }
        if has_action(frame, ActionKind::SimReset) {
            self.spawn_reset();
        }
        // Disarm is handled BEFORE the reset latch: surrendering
        // authority must never be blocked, and it needs no measurement.
        if has_action(frame, ActionKind::Disarm) {
            let Some(uplink) = self.uplink.as_mut() else {
                return Some(rejected_control(tick, RejectReason::UnknownScope));
            };
            uplink.send_disarm();
            return Some(ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results: frame
                    .actions
                    .iter()
                    .map(|action| self::action_result_for_disarm_frame(*action))
                    .collect(),
            });
        }
        // The commanded-reset latch: cached measurements inside the
        // freshness budget are pre-reset data, and the rebooting FC
        // accepts arm before its estimator converges. Suppress control
        // until the estimate stream enters a fresh source epoch and the
        // holder demonstrates neutral input.
        if self.reset_latch_blocks(frame) {
            return Some(rejected_control(tick, RejectReason::ResetInProgress));
        }
        None
    }

    /// Whether the commanded-reset latch suppresses this frame,
    /// attempting clearance first: the estimate stream must have entered
    /// a new source epoch (only the restarted FC's own measurements can
    /// do that — a stale cache keeps the engaged epoch), the frame must
    /// be neutral, and a full pose must be recoverable from the fresh
    /// stream.
    fn reset_latch_blocks(&mut self, frame: &ScopedControlFrame) -> bool {
        let Some(latch) = self.reset_latch else {
            return false;
        };
        let epoch_advanced = matches!(
            (latch.engaged_epoch, self.observed_source_epoch()),
            (Some(engaged), Some(current)) if current != engaged
        );
        if epoch_advanced && frame_is_neutral(frame) && self.current_pose().is_some() {
            tracing::info!("reset latch cleared: fresh FC stream and neutral input");
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
/// tests: the script resets the live simulator and kills any running
/// SITL FC on the machine.
#[cfg(not(test))]
fn run_reset_command() {
    let script = std::env::var("PILOTAGE_RESET_CMD").unwrap_or_else(|_| {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .map_or_else(|| ".".to_owned(), |path| path.display().to_string())
            + "/scripts/reset-flight-sim.sh"
    });
    tracing::info!(%script, "simulation reset requested from the viewer");
    if let Err(error) = std::process::Command::new(&script)
        .arg("aviate_sitl")
        .spawn()
    {
        tracing::warn!(%error, %script, "reset script failed to spawn");
    }
}
