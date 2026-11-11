//! What the flight controller says about itself, and the arm it gates.
//!
//! A flight controller that has not finished its own start refuses an arm,
//! and the host then waits out the arm's whole confirmation timeout before
//! the next try. A host that feeds [`VehicleStatus`] and sets
//! [`crate::MissionConfig::arm_when_vehicle_ready`] gets the arm request
//! held until the vehicle reports itself ready, with a typed event naming
//! the reason instead of a silent wait.

use navigate_contract::MonotonicNanos;

use super::{MissionAction, MissionEngine, MissionEvent};

/// The age past which a vehicle status no longer says the vehicle is ready
/// now, nanoseconds: a flight controller states its readiness in every
/// heartbeat, once a second.
pub const VEHICLE_STATUS_MAX_AGE_NS: u64 = 2_000_000_000;

/// One report of the vehicle's own state, as its flight controller states
/// it. The host converts it from the flight controller's link.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleStatus {
    /// Whether the flight controller reports its pre-arm checks passed: it
    /// would accept an arm now. A MAVLink flight controller says so with a
    /// HEARTBEAT `system_status` of STANDBY or ACTIVE.
    pub ready_to_arm: bool,
    /// When the report was received, on the engine's clock domain.
    pub acquired_at: MonotonicNanos,
}

/// Why the engine holds back the start of the mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartWithheldReason {
    /// No vehicle status has said, within
    /// [`VEHICLE_STATUS_MAX_AGE_NS`], that the vehicle is ready to arm.
    VehicleNotReady,
}

/// The latest vehicle status and the arm request held for it.
#[derive(Debug, Default)]
pub(super) struct VehicleReadiness {
    latest: Option<VehicleStatus>,
    held_arm: Option<MissionAction>,
}

impl MissionEngine {
    /// Records the vehicle's latest status. A report older than the one
    /// held is ignored.
    pub fn on_vehicle_status(&mut self, status: VehicleStatus) {
        let newer = self
            .vehicle
            .latest
            .is_none_or(|held| status.acquired_at >= held.acquired_at);
        if newer {
            self.vehicle.latest = Some(status);
        }
    }

    /// Whether the vehicle reported itself ready to arm within
    /// [`VEHICLE_STATUS_MAX_AGE_NS`] of `now`.
    pub(super) fn vehicle_ready(&self, now: MonotonicNanos) -> bool {
        self.vehicle.latest.is_some_and(|status| {
            status.ready_to_arm
                && now.as_nanos().saturating_sub(status.acquired_at.as_nanos())
                    <= VEHICLE_STATUS_MAX_AGE_NS
        })
    }

    /// The arm request the core emitted at `now`: sent now, or held until
    /// the vehicle is ready when the configuration asks for that.
    pub(super) fn gate_arm(
        &mut self,
        action: MissionAction,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<MissionAction> {
        if !self.config.arm_when_vehicle_ready || self.vehicle_ready(now) {
            events.push(MissionEvent::ArmRequested {
                action_id: action.action_id,
            });
            return Some(action);
        }
        self.vehicle.held_arm = Some(action);
        events.push(MissionEvent::StartWithheld {
            reason: StartWithheldReason::VehicleNotReady,
        });
        None
    }

    /// The held arm request, once the vehicle is ready at `now`.
    pub(super) fn release_held_arm(
        &mut self,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<MissionAction> {
        if self.vehicle.held_arm.is_none() || !self.vehicle_ready(now) {
            return None;
        }
        let action = self.vehicle.held_arm.take()?;
        events.push(MissionEvent::ArmRequested {
            action_id: action.action_id,
        });
        Some(action)
    }

    /// Whether an arm request is out to the vehicle: emitted, and not held.
    pub(super) fn arm_request_sent(&self) -> bool {
        self.outstanding_arm.is_some() && self.vehicle.held_arm.is_none()
    }

    /// Forgets a held arm request whose directive the core withdrew.
    pub(super) fn drop_held_arm(&mut self) {
        self.vehicle.held_arm = None;
    }
}
