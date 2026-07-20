//! Gimbal pointing-frame application for the `vehicle.gimbal` scope:
//! structural validation, the typed claim-denial surface, and demand
//! extraction into the gimbal-manager command path.

use std::time::Duration;

use pilotage_adapter_api::{ApplyOutcome, Disposition, RejectReason};
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, ScopedControlFrame};
use pilotage_timing::SimTick;

use super::control::rejected_control;
use super::{GIMBAL_NEUTRAL_BUTTON, PITCH_AXIS, Px4Adapter, YAW_AXIS};
use crate::gimbal::CMD_GIMBAL_CONFIGURE;

/// How long a CONFIGURE denial keeps rejecting pointing frames. The
/// claim is re-asserted every second while demands flow, so a live
/// denial refreshes itself and a stale one ages out instead of
/// blocking the scope forever.
const CLAIM_DENIAL_FRESHNESS: Duration = Duration::from_secs(5);

impl Px4Adapter {
    /// Applies one `vehicle.gimbal` frame: neutral-reset edges and
    /// pitch/yaw LOS rate demands. Pointing is deliberately outside
    /// the flight gate chain — a gimbal cannot fly the vehicle, and
    /// its scope is leased and fenced independently — but a fresh
    /// primary-control denial from the FC rejects frames loudly
    /// because PX4 ignores non-primary demands silently.
    pub(super) fn apply_gimbal(
        &mut self,
        frame: &ScopedControlFrame,
        tick: SimTick,
    ) -> ApplyOutcome {
        if frame.vehicle != self.vehicle {
            return rejected_control(tick, RejectReason::UnknownVehicle);
        }
        let known = [LogicalAxisId::new(PITCH_AXIS), LogicalAxisId::new(YAW_AXIS)];
        for (axis, _) in &frame.payload.axes {
            if !known.contains(axis) {
                return rejected_control(tick, RejectReason::UnknownAxis);
            }
        }
        let denial = self.fresh_claim_denial();
        let Some(gimbal) = self.gimbal.as_mut() else {
            return rejected_control(tick, RejectReason::UnknownScope);
        };
        if let Some(result) = denial {
            return rejected_control(
                tick,
                RejectReason::Other(format!(
                    "gimbal primary-control claim denied by the FC (MAV_RESULT {result})"
                )),
            );
        }
        // A send that never reached the wire (a full or closed lane)
        // must not report as applied. Track every command/demand this
        // frame issued.
        let mut delivered = true;
        for (button, edge) in &frame.payload.edges {
            if *edge == ButtonEdge::Pressed
                && *button == LogicalButtonId::new(GIMBAL_NEUTRAL_BUTTON)
            {
                delivered &= gimbal.neutral();
            }
        }
        let mut pitch = 0.0_f32;
        let mut yaw = 0.0_f32;
        let mut transformed = false;
        for (axis, value) in &frame.payload.axes {
            let clamped = if value.is_nan() {
                0.0
            } else {
                value.clamp(-1.0, 1.0)
            };
            transformed |= clamped != *value;
            if *axis == LogicalAxisId::new(PITCH_AXIS) {
                pitch = clamped;
            } else {
                yaw = clamped;
            }
        }
        if !frame.payload.axes.is_empty() {
            delivered &= gimbal.rate_demand(pitch, yaw);
        }
        if !delivered {
            return rejected_control(
                tick,
                RejectReason::Other("gimbal command lane full; demand not sent".to_owned()),
            );
        }
        ApplyOutcome {
            tick,
            disposition: if transformed {
                Disposition::Transformed
            } else {
                Disposition::Accepted
            },
        }
    }

    /// A recent CONFIGURE denial from the FC, if one is cached on the
    /// receive link. Reads the dedicated CONFIGURE ack slot so an
    /// unrelated later ack (a periodic SET_MESSAGE_INTERVAL, MAV_CMD
    /// 511) cannot bury a claim denial and let ignored gimbal demands
    /// report as accepted.
    fn fresh_claim_denial(&self) -> Option<u8> {
        let source = self.estimate.as_ref()?;
        let latest = source.state.lock().ok()?;
        let ack = latest.gimbal_configure_ack?;
        (ack.command == CMD_GIMBAL_CONFIGURE
            && ack.result != 0
            && ack.received_at.elapsed() < CLAIM_DENIAL_FRESHNESS)
            .then_some(ack.result)
    }
}
