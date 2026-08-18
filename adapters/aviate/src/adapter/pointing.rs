//! The gimbal pointing scope for a producer-rendered payload view.
//!
//! The scope's wire vocabulary is a RATE (rad/s, GIM-01), but the
//! producer this adapter drives consumes an ABSOLUTE pointing angle: it
//! renders a view, it does not run a servo. This module owns that
//! impedance conversion — it integrates the commanded rate into a held
//! pan/tilt within the producer's declared travel, and publishes the
//! resulting angle.
//!
//! Safety difference from a MAVLink gimbal, stated plainly: a
//! gimbal-manager FC zeroes a stale nonzero rate on its own timeout, so
//! a host's link-loss stop there is best-effort with an independent net
//! behind it. A rendered view has NO such timeout — this adapter's stop
//! is the ONLY mechanism. It therefore freezes the pointing by
//! republishing the current angle (a failsafe stops the camera where it
//! is; it does not slew it to level) and refuses to report success it
//! did not achieve.

use std::time::{Duration, Instant};

use pilotage_sim_video::wire::BridgeCameraCommand;

/// Loopback port the in-simulator camera plugin dials.
pub(crate) const XPLANE_CAMERA_PORT: u16 = 45990;

/// One zoom detent: the producer's field of view and the calibration it
/// publishes for frames captured at that detent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ZoomDetent {
    /// Horizontal field of view, degrees. The producer owns the
    /// enactment; this table documents the ladder the adapter steps
    /// through and pins it against the producer's own.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) field_of_view_deg: f32,
    /// The calibration identity frames carry at this detent. Consumed
    /// by the test that pins detent calibrations distinct; the frame
    /// stamper cannot bind it per frame until the producer reports the
    /// detent each picture was captured at (see `camera.rs`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) calibration_id: u32,
}

/// The detent table, mirroring the producer's own. Entry 0 is the widest
/// framing, which is where the producer starts.
pub(crate) const ZOOM_DETENTS: [ZoomDetent; 4] = [
    ZoomDetent {
        field_of_view_deg: 100.0,
        calibration_id: 0x5850_0001,
    },
    ZoomDetent {
        field_of_view_deg: 60.0,
        calibration_id: 0x5850_0002,
    },
    ZoomDetent {
        field_of_view_deg: 30.0,
        calibration_id: 0x5850_0003,
    },
    ZoomDetent {
        field_of_view_deg: 12.0,
        calibration_id: 0x5850_0004,
    },
];

/// Producer travel limits, radians. The adapter clamps to these so a
/// demand it cannot enact is reported as constrained, never as applied.
pub(crate) const PAN_LIMIT_RAD: f32 = std::f32::consts::PI;
pub(crate) const TILT_MIN_RAD: f32 = -std::f32::consts::FRAC_PI_2;
pub(crate) const TILT_MAX_RAD: f32 = std::f32::consts::FRAC_PI_6;

/// The rate envelope this adapter enacts. Advertised verbatim, so a
/// client scales its normalized stick against the rate actually applied.
pub(crate) const MAX_PITCH_RATE_RPS: f32 = 0.8;
pub(crate) const MAX_YAW_RATE_RPS: f32 = 0.8;

/// The interval one rate demand is integrated over. The control plane
/// delivers frames at a fixed cadence, and a wall-clock delta would let
/// a stalled client slew the payload on resume.
const INTEGRATION_STEP_S: f32 = 1.0 / 30.0;

/// Producer camera modes, matching `BridgeCameraCommand.mode`. The
/// values are 1-based: proto3 omits a zero-valued scalar, so a
/// zero-numbered mode could never travel and the producer would keep
/// whichever view it was already rendering.
const MODE_FPV: u32 = 1;
const MODE_GIMBAL: u32 = 2;

/// How long the payload view stays selected after the last pointing
/// command.
///
/// The producer renders ONE view, so showing the payload means NOT
/// showing the forward view. The operator's gimbal control is a
/// quasimode — held while aiming, released otherwise — so the view
/// follows it and returns to the vehicle's forward camera when aiming
/// stops. Without this the first pointing command of a session (or a
/// link-loss freeze) would leave the forward feed dark for good.
const PAYLOAD_VIEW_HOLD: Duration = Duration::from_secs(2);

/// The commanded pointing state of a producer-rendered payload view.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PointingState {
    pan_rad: f32,
    tilt_rad: f32,
    zoom_detent: u32,
    /// When the payload view was last commanded; `None` means the
    /// producer shows the vehicle's forward view.
    aimed_at: Option<Instant>,
    /// The view the producer was last told to render, so the adapter
    /// republishes exactly when that changes rather than every tick.
    /// `None` until the first publish: a producer outlives a session
    /// and keeps whatever view the PREVIOUS one left it on, so the
    /// adapter states the view rather than assuming it.
    published_mode: Option<u32>,
}

impl Default for PointingState {
    fn default() -> Self {
        Self {
            pan_rad: 0.0,
            tilt_rad: 0.0,
            zoom_detent: 0,
            aimed_at: None,
            published_mode: None,
        }
    }
}

impl PointingState {
    /// Integrates one rate demand (rad/s) into the held pointing,
    /// clamped to the producer's travel. Returns `true` when the demand
    /// was clamped, so the caller can report a constrained disposition
    /// rather than claiming the full demand was enacted.
    pub(crate) fn integrate(&mut self, pitch_rate_rps: f32, yaw_rate_rps: f32) -> bool {
        let pitch_rate = sanitize(pitch_rate_rps, MAX_PITCH_RATE_RPS);
        let yaw_rate = sanitize(yaw_rate_rps, MAX_YAW_RATE_RPS);
        let clamped_rate = pitch_rate.1 || yaw_rate.1;

        let pan = self.pan_rad + yaw_rate.0 * INTEGRATION_STEP_S;
        let tilt = self.tilt_rad + pitch_rate.0 * INTEGRATION_STEP_S;
        let clamped_travel = !(-PAN_LIMIT_RAD..=PAN_LIMIT_RAD).contains(&pan)
            || !(TILT_MIN_RAD..=TILT_MAX_RAD).contains(&tilt);
        self.pan_rad = pan.clamp(-PAN_LIMIT_RAD, PAN_LIMIT_RAD);
        self.tilt_rad = tilt.clamp(TILT_MIN_RAD, TILT_MAX_RAD);
        clamped_rate || clamped_travel
    }

    /// Returns the pointing to its stowed orientation. The zoom detent
    /// is deliberately kept: recentering aims the payload, it does not
    /// change which camera model the frames carry.
    pub(crate) fn recenter(&mut self) {
        self.pan_rad = 0.0;
        self.tilt_rad = 0.0;
    }

    /// Steps one detent toward a narrower field of view. Returns `false`
    /// at the narrowest detent, so the caller reports the refusal
    /// instead of silently swallowing the press.
    pub(crate) fn zoom_in(&mut self) -> bool {
        let last = u32::try_from(ZOOM_DETENTS.len().saturating_sub(1)).unwrap_or(0);
        if self.zoom_detent >= last {
            return false;
        }
        self.zoom_detent = self.zoom_detent.saturating_add(1);
        true
    }

    /// Steps one detent toward a wider field of view. Returns `false` at
    /// the widest detent.
    pub(crate) fn zoom_out(&mut self) -> bool {
        if self.zoom_detent == 0 {
            return false;
        }
        self.zoom_detent = self.zoom_detent.saturating_sub(1);
        true
    }

    /// The detent currently in effect.
    #[cfg(test)]
    pub(crate) fn detent(&self) -> ZoomDetent {
        ZOOM_DETENTS
            .get(self.zoom_detent as usize)
            .copied()
            .unwrap_or(ZOOM_DETENTS[0])
    }

    /// Marks the payload view as commanded, so the producer shows it
    /// for as long as the operator keeps aiming.
    pub(crate) fn aim(&mut self) {
        self.aimed_at = Some(Instant::now());
    }

    /// Which view the producer should render right now.
    pub(crate) fn mode(&self) -> u32 {
        match self.aimed_at {
            Some(at) if at.elapsed() < PAYLOAD_VIEW_HOLD => MODE_GIMBAL,
            _ => MODE_FPV,
        }
    }

    /// The producer command for the current state.
    pub(crate) fn command(&self) -> BridgeCameraCommand {
        BridgeCameraCommand {
            mode: self.mode(),
            pan_rad: self.pan_rad,
            tilt_rad: self.tilt_rad,
            zoom_detent: self.zoom_detent,
        }
    }
}

/// Clamps a rate to the advertised envelope, mapping a non-finite value
/// to zero (the safe side: no motion rather than a slew from a NaN).
/// Returns the clamped value and whether clamping changed it.
fn sanitize(value: f32, limit: f32) -> (f32, bool) {
    if !value.is_finite() {
        return (0.0, true);
    }
    let clamped = value.clamp(-limit, limit);
    (clamped, (clamped - value).abs() > f32::EPSILON)
}

/// Enacts the gimbal scope's discrete actions, one explicit result per
/// action: a press is answered, never silently dropped (CTRL-01).
fn process_pointing_actions(
    actions: &[pilotage_protocol::ControlAction],
    pointing: &mut PointingState,
) -> Vec<pilotage_adapter_api::ActionResult> {
    use pilotage_adapter_api::ActionResult;
    use pilotage_protocol::ControlAction;

    actions
        .iter()
        .map(|action| match *action {
            ControlAction::GimbalRecenter => {
                pointing.recenter();
                ActionResult::accepted(*action)
            }
            // A refused detent step is reported, never swallowed: the
            // operator pressed a key and is owed an answer.
            ControlAction::CameraZoomIn => {
                if pointing.zoom_in() {
                    ActionResult::accepted(*action)
                } else {
                    ActionResult::rejected(*action, "already at the narrowest detent")
                }
            }
            ControlAction::CameraZoomOut => {
                if pointing.zoom_out() {
                    ActionResult::accepted(*action)
                } else {
                    ActionResult::rejected(*action, "already at the widest detent")
                }
            }
            other => ActionResult::rejected(other, "not supported on the gimbal scope"),
        })
        .collect()
}

impl PointingState {
    /// Records the view the producer has been told to render.
    fn note_published(&mut self) {
        self.published_mode = Some(self.mode());
    }

    /// Whether the producer may be rendering a different view than the
    /// commanded state calls for — including the case where it has
    /// never been told, and is therefore on whatever the last session
    /// left it on.
    fn view_is_stale(&self) -> bool {
        self.published_mode != Some(self.mode())
    }
}

impl super::AviateAdapter {
    /// Enacts one gimbal-scope frame: recenter and zoom actions, then the
    /// integrated pointing. The scope consumes TYPED commands only — a
    /// legacy payload is translated at the host's compatibility boundary,
    /// never reinterpreted here.
    pub(super) fn apply_gimbal(
        &mut self,
        frame: &pilotage_protocol::ScopedControlFrame,
        tick: pilotage_timing::SimTick,
    ) -> pilotage_adapter_api::ApplyOutcome {
        use pilotage_adapter_api::{Disposition, RejectReason};
        use pilotage_protocol::ControlIntent;

        if frame.vehicle != self.vehicle {
            return super::rejected_control(tick, RejectReason::UnknownVehicle);
        }
        if frame.carries_payload() || !frame.carries_typed() {
            return super::rejected_control(
                tick,
                RejectReason::Other("the gimbal scope consumes typed commands only".to_owned()),
            );
        }
        let Some(pointing) = self.pointing.as_mut() else {
            return super::rejected_control(tick, RejectReason::UnknownScope);
        };

        // Any command on this scope selects the payload view; it
        // reverts to the forward view once aiming stops.
        pointing.aim();
        let action_results = process_pointing_actions(&frame.actions, pointing);

        let mut constrained = false;
        match frame.intent {
            Some(ControlIntent::GimbalRate(rate)) => {
                constrained = pointing.integrate(rate.pitch_rate, rate.yaw_rate);
            }
            Some(_) => {
                return super::rejected_control(
                    tick,
                    RejectReason::Other(
                        "the gimbal scope consumes gimbal-rate intents only".to_owned(),
                    ),
                );
            }
            None => {}
        }

        let command = pointing.command();
        pointing.note_published();
        if !self.publish_camera_command(command) {
            return super::rejected_control(
                tick,
                RejectReason::Other("the camera producer link is down".to_owned()),
            );
        }
        pilotage_adapter_api::ApplyOutcome {
            tick,
            disposition: if constrained {
                Disposition::Constrained
            } else {
                Disposition::Accepted
            },
            action_results,
        }
    }

    /// Freezes or releases the payload pointing for the gimbal scope's
    /// link-loss policy. Engaging republishes the CURRENT angle: a
    /// failsafe stops the camera where it is. There is no producer-side
    /// setpoint timeout behind this, so a refused publish must be
    /// reported, never assumed.
    pub(super) fn enact_gimbal_link_loss(
        &mut self,
        engaging: bool,
    ) -> Result<(), pilotage_adapter_api::LinkLossEnactError> {
        let Some(pointing) = self.pointing else {
            return Err(pilotage_adapter_api::LinkLossEnactError::NoActuationChannel);
        };
        if !engaging {
            // Clearing restores normal control; the pointing already
            // holds where the failsafe left it.
            return Ok(());
        }
        if self.publish_camera_command(pointing.command()) {
            Ok(())
        } else {
            Err(pilotage_adapter_api::LinkLossEnactError::ChannelRejected {
                detail: "the pointing freeze could not be published".to_owned(),
            })
        }
    }

    /// Returns the producer to the vehicle's forward view once aiming
    /// stops, so one rendered view does not stay stuck on the payload
    /// after the operator lets go. Runs on the telemetry tick, and
    /// publishes only on a change.
    pub(super) fn maintain_camera_view(&mut self) {
        let Some(pointing) = self.pointing.as_ref() else {
            return;
        };
        if !pointing.view_is_stale() {
            return;
        }
        let command = pointing.command();
        let published = self.publish_camera_command(command);
        // The producer renders ONE view, so which view it is showing is
        // operator-visible state, not a detail. It changes rarely, and a
        // change that did not reach the producer is why a feed goes dark.
        tracing::info!(
            mode = command.mode,
            published,
            "payload view selection sent to the producer"
        );
        if published && let Some(pointing) = self.pointing.as_mut() {
            pointing.note_published();
        }
    }

    /// Publishes one camera command to the producer. `false` means the
    /// producer link is gone, so the caller reports a rejection instead
    /// of an enactment that never left.
    fn publish_camera_command(&self, command: BridgeCameraCommand) -> bool {
        self.camera_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.try_send_camera_command(command))
    }
}

#[cfg(test)]
mod tests;
