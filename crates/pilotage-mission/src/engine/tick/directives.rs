//! The flight handler's answer to each directive the core emits: the vehicle action it
//! becomes, or the receipt that refuses it.

use pilotage_mission_core::{
    DirectiveReceipt, FlightAction as CoreFlightAction, MissionDirective, ReceiptResult,
};
use pilotage_protocol::ControlAction;

use super::super::{MissionAction, MissionEngine, MissionEvent};

impl MissionEngine {
    pub(super) fn arm_receipt_event(
        &self,
        action_id: u32,
        result: &ReceiptResult,
        events: &mut Vec<MissionEvent>,
    ) {
        let action_id = u64::from(action_id);
        match result {
            ReceiptResult::Succeeded {} => events.push(MissionEvent::ArmAccepted { action_id }),
            ReceiptResult::Retryable { .. } => events.push(MissionEvent::ArmRejected { action_id }),
            ReceiptResult::Refused { .. } | ReceiptResult::Failed { .. } => {}
        }
    }

    pub(super) fn interpret_directive(
        &mut self,
        directive: &MissionDirective,
        events: &mut Vec<MissionEvent>,
    ) -> Option<MissionAction> {
        let MissionDirective::Flight(directive) = directive else {
            let detail = match directive {
                MissionDirective::Bound(bound) => format!(
                    "the operational host has no executor {}",
                    bound.action.executor
                ),
                _ => "the operational host does not execute trial directives".to_owned(),
            };
            self.queue_receipt(
                directive.context().action_id,
                ReceiptResult::Refused { detail },
            );
            return None;
        };
        match &directive.action {
            CoreFlightAction::Arm {} => {
                let action_id = directive.context.action_id;
                self.active_action = Some(directive.action.clone());
                self.outstanding_arm = Some(action_id);
                events.push(MissionEvent::ArmRequested {
                    action_id: u64::from(action_id.get()),
                });
                Some(MissionAction {
                    action: ControlAction::Arm,
                    action_id: u64::from(action_id.get()),
                })
            }
            CoreFlightAction::Climb { .. } => {
                self.active_action = Some(directive.action.clone());
                self.queue_receipt(directive.context.action_id, ReceiptResult::Succeeded {});
                if self.config.cruise_height_m > 0.0 {
                    events.push(MissionEvent::ClimbStarted);
                }
                None
            }
            CoreFlightAction::FollowPlan { plan } if *plan == self.plan_reference => {
                self.active_action = Some(directive.action.clone());
                self.queue_receipt(directive.context.action_id, ReceiptResult::Succeeded {});
                events.push(MissionEvent::EnrouteStarted);
                None
            }
            // A segment continues the one plan the vehicle flies, from the waypoint it
            // flies toward now.
            CoreFlightAction::FollowSegment {
                plan,
                first_waypoint,
                ..
            } if *plan == self.plan_reference
                && usize::try_from(*first_waypoint).ok() == Some(self.waypoints_captured()) =>
            {
                if !self.following() {
                    events.push(MissionEvent::EnrouteStarted);
                }
                self.active_action = Some(directive.action.clone());
                self.queue_receipt(directive.context.action_id, ReceiptResult::Succeeded {});
                None
            }
            action => {
                self.queue_receipt(
                    directive.context.action_id,
                    ReceiptResult::Refused {
                        detail: format!("the operational host cannot execute {action:?}"),
                    },
                );
                None
            }
        }
    }

    /// Whether the vehicle follows the plan, whole or a segment of it.
    pub(super) fn following(&self) -> bool {
        matches!(
            self.active_action,
            Some(CoreFlightAction::FollowPlan { .. } | CoreFlightAction::FollowSegment { .. })
        )
    }

    /// How many of the plan's waypoints are captured, from its first.
    pub(super) fn waypoints_captured(&self) -> usize {
        let total = self.execution.plan().waypoints.len();
        total.saturating_sub(self.execution.remaining_waypoints().len())
    }

    pub(super) fn queue_receipt(
        &mut self,
        action_id: pilotage_mission_core::ActionId,
        result: ReceiptResult,
    ) {
        self.pending_receipt = Some(DirectiveReceipt { action_id, result });
    }
}
