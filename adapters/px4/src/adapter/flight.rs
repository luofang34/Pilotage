//! The airframe split of the flight scope: which uplink a PX4 instance
//! gets, and the fixed-wing control path beside the multicopter one in
//! `control.rs`.

use pilotage_adapter_api::{ActionResult, ApplyOutcome, Disposition, FcCommandAck, RejectReason};
use pilotage_mavlink::LinkConfig;
use pilotage_protocol::{ActionKind, ControlIntent, ScopedControlFrame};
use pilotage_timing::SimTick;

use super::Px4Adapter;
use super::control::rejected_control;
use crate::config::{Px4Airframe, Px4Config};
use crate::fixed_wing::{ArmRefusal, FixedWingUplink, IntentDisposition};
use crate::uplink::Px4Uplink;

/// Binds the airframe's command uplink; exactly one of the pair is
/// present on success. A failed bind degrades to telemetry-only rather
/// than failing the adapter.
pub(super) fn flight_uplinks(
    config: &Px4Config,
    link: &LinkConfig,
) -> (Option<Px4Uplink>, Option<FixedWingUplink>) {
    let bound = match config.airframe {
        Px4Airframe::Multicopter => {
            Px4Uplink::new(config.profile(), config.command_endpoint).map(|mut uplink| {
                uplink.set_expected_source(link.system_id, link.component_id);
                (Some(uplink), None)
            })
        }
        Px4Airframe::FixedWing => FixedWingUplink::new(
            config.profile(),
            config.command_endpoint,
            link.system_id,
            link.component_id,
        )
        .map(|uplink| (None, Some(uplink))),
    };
    bound.unwrap_or_else(|error| {
        tracing::warn!(%error, "PX4 uplink unavailable; telemetry-only");
        (None, None)
    })
}

impl Px4Adapter {
    /// The FC's verdict on the latest arm/disarm, from whichever uplink
    /// this airframe uses.
    pub(super) fn flight_arm_ack(&self) -> Option<FcCommandAck> {
        match (&self.uplink, &self.fixed_wing) {
            (Some(uplink), _) => uplink.last_arm_ack(),
            (None, Some(fixed_wing)) => fixed_wing.last_arm_ack(),
            (None, None) => None,
        }
    }

    /// Periodic uplink upkeep at telemetry cadence. The fixed-wing
    /// takeoff handover needs the present height estimate.
    pub(super) fn maintain_flight_uplink(&mut self) {
        let armed = self.arm.report().map(|report| report.armed);
        if let Some(uplink) = self.uplink.as_mut() {
            uplink.maintain();
            uplink.observe_fc_armed(armed);
        }
        if self.fixed_wing.is_some() {
            let down_m = self.current_pose().map(|pose| pose.pos_ned_m[2]);
            if let Some(fixed_wing) = self.fixed_wing.as_mut() {
                fixed_wing.maintain(down_m);
            }
        }
    }

    /// The fixed-wing flight path once the gates have admitted a frame:
    /// arm starts the takeoff sequence, and a velocity intent becomes
    /// Position-mode sticks (neutral until the takeoff handover).
    pub(super) fn apply_fixed_wing_flight(
        &mut self,
        frame: &ScopedControlFrame,
        tick: SimTick,
    ) -> ApplyOutcome {
        let Some(pos) = self.current_pose().map(|pose| pose.pos_ned_m) else {
            return rejected_control(tick, RejectReason::MeasurementUnavailable);
        };
        let Some(uplink) = self.fixed_wing.as_mut() else {
            return rejected_control(tick, RejectReason::UnknownScope);
        };
        let action_results = frame
            .actions
            .iter()
            .map(|action| match action.kind() {
                ActionKind::Arm => match uplink.begin_arm(pos[2]) {
                    Ok(()) => ActionResult::accepted(*action),
                    Err(ArmRefusal::Parameter(refusal)) => {
                        ActionResult::rejected(*action, refusal.to_string())
                    }
                },
                _ => ActionResult::rejected(*action, "not supported on the flight scope"),
            })
            .collect();
        let Some(ControlIntent::Velocity(velocity)) = frame.intent else {
            return ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results,
            };
        };
        let disposition = match uplink.apply_velocity(&velocity) {
            IntentDisposition::Flown => Disposition::Accepted,
            IntentDisposition::Constrained => Disposition::Constrained,
            IntentDisposition::UplinkIdle => Disposition::Rejected(RejectReason::UplinkIdle),
        };
        ApplyOutcome {
            tick,
            disposition,
            action_results,
        }
    }
}
