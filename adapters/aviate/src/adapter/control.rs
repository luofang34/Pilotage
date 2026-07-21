//! Flight-control input helpers, control gating, and reset handling.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    ApplyOutcome, Disposition, RejectReason, payload_satisfies_neutral_activation,
};
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, ScopedControlFrame};
use pilotage_timing::SimTick;

use super::{
    AviateAdapter, DISARM_BUTTON, PITCH_AXIS, RESET_BUTTON, ROLL_AXIS, THROTTLE_AXIS, YAW_AXIS,
};

/// Reset clearance uses the same 5%-of-full-stick scale as host recovery.
const RESET_CLEAR_DEADBAND_MILLI: u32 = 50;
const FLIGHT_AXES: [LogicalAxisId; 4] = [
    LogicalAxisId::new(ROLL_AXIS),
    LogicalAxisId::new(PITCH_AXIS),
    LogicalAxisId::new(THROTTLE_AXIS),
    LogicalAxisId::new(YAW_AXIS),
];

/// The engaged commanded-reset latch: the estimate stream's source epoch
/// observed at engagement. `engaged_epoch` is `None` when no estimate
/// stream was observable at that moment; the epoch-advance clearance can
/// then never be satisfied, which fails closed (a profile without an
/// estimate stream cannot arm anyway).
#[derive(Debug, Clone, Copy)]
pub(super) struct ResetLatch {
    engaged_epoch: Option<u32>,
}

/// Reset clearance uses the canonical full-coverage neutral activation.
fn frame_is_neutral(frame: &ScopedControlFrame) -> bool {
    payload_satisfies_neutral_activation(&frame.payload, &FLIGHT_AXES, RESET_CLEAR_DEADBAND_MILLI)
}

pub(super) fn flight_button_pressed(frame: &ScopedControlFrame, button: u16) -> bool {
    frame.payload.edges.iter().any(|(candidate, edge)| {
        *edge == ButtonEdge::Pressed && *candidate == LogicalButtonId::new(button)
    })
}

pub(super) fn rejected_control(tick: SimTick, reason: RejectReason) -> ApplyOutcome {
    ApplyOutcome {
        tick,
        disposition: Disposition::Rejected(reason),
    }
}

pub(super) fn normalized_flight_sticks(frame: &ScopedControlFrame) -> ([f32; 4], bool) {
    let mut sticks = [0.0_f32; 4];
    let mut transformed = false;
    for (axis, value) in &frame.payload.axes {
        let clamped = if value.is_nan() {
            0.0
        } else {
            value.clamp(-1.0, 1.0)
        };
        transformed |= clamped != *value;
        sticks[usize::from(axis.as_u16().min(3))] = clamped;
    }
    (sticks, transformed)
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

    /// The pre-pose gate chain for one flight frame: structural checks,
    /// the link-loss latch, reset/disarm button handling, and the
    /// commanded-reset latch. `Some` is the early outcome; `None` lets
    /// the caller proceed to measurement-dependent control.
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
        // The link-loss latch: while a policy is engaged the FC holds its
        // policy state (braked hover) and ordinary frames are suppressed,
        // so a newly granted holder with deflected sticks cannot fly the
        // vehicle out of it. The host clears the latch only after the
        // holder demonstrates neutral input.
        if self.link_loss_policy.contains_key(&frame.scope) {
            return Some(rejected_control(tick, RejectReason::LinkLossEngaged));
        }
        if flight_button_pressed(frame, RESET_BUTTON) {
            self.spawn_reset();
        }
        // Disarm is checked BEFORE the reset latch: surrendering
        // authority must never be blocked, and it needs no measurement.
        if flight_button_pressed(frame, DISARM_BUTTON) {
            let Some(uplink) = self.uplink.as_mut() else {
                return Some(rejected_control(tick, RejectReason::UnknownScope));
            };
            uplink.send_disarm();
            return Some(ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
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
