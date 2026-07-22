//! Gimbal pointing-frame application for the `vehicle.gimbal` scope:
//! structural validation, the typed claim-denial surface, and typed
//! gimbal-rate/recenter consumption into the gimbal-manager command path.

use std::time::Duration;

use pilotage_adapter_api::{ActionResult, ApplyOutcome, Disposition, RejectReason};
use pilotage_protocol::{ActionKind, ControlIntent, ScopedControlFrame};
use pilotage_timing::SimTick;

use super::Px4Adapter;
use super::control::rejected_control;
use crate::gimbal::{CMD_GIMBAL_CONFIGURE, MAX_PITCH_RATE_RPS, MAX_YAW_RATE_RPS};

/// How long a CONFIGURE denial keeps rejecting pointing frames. The
/// claim is re-asserted every second while demands flow, so a live
/// denial refreshes itself and a stale one ages out instead of
/// blocking the scope forever.
const CLAIM_DENIAL_FRESHNESS: Duration = Duration::from_secs(5);

impl Px4Adapter {
    /// Applies one `vehicle.gimbal` frame: the typed recenter action and
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
        // Typed-only consumption: the session host translates any legacy
        // payload at its compatibility boundary before delivery.
        if frame.carries_payload() || !frame.carries_typed() {
            return rejected_control(
                tick,
                RejectReason::Other("the gimbal scope consumes typed commands only".to_owned()),
            );
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
        let action_results: Vec<ActionResult> = frame
            .actions
            .iter()
            .map(|action| match action.kind() {
                ActionKind::GimbalRecenter => {
                    let sent = gimbal.neutral();
                    delivered &= sent;
                    if sent {
                        ActionResult::accepted(*action)
                    } else {
                        ActionResult::rejected(*action, "gimbal command lane full")
                    }
                }
                // Nothing else is advertised for this scope; the session
                // rejects it before delivery — defensive, not reachable.
                _ => ActionResult::rejected(*action, "not supported on the gimbal scope"),
            })
            .collect();
        let mut constrained = false;
        if let Some(intent) = frame.intent {
            let ControlIntent::GimbalRate(rate) = intent else {
                return rejected_control(
                    tick,
                    RejectReason::Other(
                        "the gimbal scope consumes gimbal-rate intents only".to_owned(),
                    ),
                );
            };
            let (pitch, yaw, clamped) = normalized_rate_demand(&rate);
            constrained = clamped;
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
            disposition: if constrained {
                Disposition::Constrained
            } else {
                Disposition::Accepted
            },
            action_results,
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

/// Converts a typed gimbal rate (rad/s inside the advertised envelope) to
/// the normalized demand the command path consumes: the exact inverse of
/// the envelope scaling a client applies. Out-of-envelope values clamp.
fn normalized_rate_demand(rate: &pilotage_protocol::GimbalRateIntent) -> (f32, f32, bool) {
    let normalize = |value: f32, limit: f32| {
        let normalized = value / limit;
        let clamped = if normalized.is_nan() {
            0.0
        } else {
            normalized.clamp(-1.0, 1.0)
        };
        (clamped, clamped != normalized)
    };
    let (pitch, pitch_clamped) = normalize(rate.pitch_rate, MAX_PITCH_RATE_RPS);
    let (yaw, yaw_clamped) = normalize(rate.yaw_rate, MAX_YAW_RATE_RPS);
    (pitch, yaw, pitch_clamped || yaw_clamped)
}
