//! What the flight controller says about itself, and the arm it gates.
//!
//! A flight controller that has not finished its own start refuses an arm,
//! and the host then waits out the arm's whole confirmation timeout before
//! the next try. A host that feeds [`VehicleStatus`] and sets
//! [`crate::MissionConfig::arm_when_vehicle_ready`] gets the arm request
//! held until the vehicle reports itself ready, with a typed event naming
//! the reason instead of a silent wait.

use navigate_contract::{MonotonicNanos, NavigationSolution};
use pilotage_mission_core::VehicleLifecycleState;

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
    /// Whether the flight controller's own estimate has a valid position,
    /// or `None` when it does not say. Without satellite navigation it
    /// usually has none, while its attitude, height, and airspeed stay
    /// valid.
    pub position_valid: Option<bool>,
    /// The flight controller's height above its local origin, metres, when
    /// its estimate states a valid height.
    pub height_above_origin_m: Option<f64>,
    /// The flight controller's indicated airspeed, m/s, when it measures
    /// one.
    pub airspeed_mps: Option<f64>,
    /// When the report was received, on the engine's clock domain.
    pub acquired_at: MonotonicNanos,
}

impl VehicleStatus {
    /// A report that states readiness only, received at `acquired_at`.
    #[must_use]
    pub const fn ready(ready_to_arm: bool, acquired_at: MonotonicNanos) -> Self {
        Self {
            ready_to_arm,
            position_valid: None,
            height_above_origin_m: None,
            airspeed_mps: None,
            acquired_at,
        }
    }
}

/// When a vehicle counts as launched by its flight controller's own
/// measurements, with no position needed: the airframe's numbers, set by
/// the host. Either one met is a launch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaunchDetection {
    /// The height above the flight controller's local origin at which the
    /// vehicle is airborne, metres.
    pub min_height_m: f64,
    /// The indicated airspeed at which a fixed wing is flying, m/s: above
    /// its stall speed, so no taxi reaches it.
    pub min_airspeed_mps: f64,
}

impl LaunchDetection {
    /// Whether `status` shows a launched vehicle.
    #[must_use]
    pub fn launched(&self, status: &VehicleStatus) -> bool {
        status
            .height_above_origin_m
            .is_some_and(|height| height >= self.min_height_m)
            || status
                .airspeed_mps
                .is_some_and(|airspeed| airspeed >= self.min_airspeed_mps)
    }
}

/// Why the engine holds back the start of the mission.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum StartWithheldReason {
    /// No vehicle status has said, within
    /// [`VEHICLE_STATUS_MAX_AGE_NS`], that the vehicle is ready to arm.
    VehicleNotReady,
    /// The ownship source has given nothing yet.
    NavigationNotStarted,
    /// The latest navigation solution is older than the source's age
    /// limit.
    NavigationStale,
    /// The navigator grades its solution unusable.
    NavigationUnusable,
    /// The navigator's horizontal uncertainty is above the limit the
    /// mission flies under.
    NavigationTooUncertain {
        /// The solution's 1-sigma horizontal uncertainty, metres.
        horizontal_1sigma_m: f64,
        /// The limit, metres.
        limit_m: f64,
    },
}

/// The latest vehicle status and the arm request held for it.
#[derive(Debug, Default)]
pub(super) struct VehicleReadiness {
    latest: Option<VehicleStatus>,
    held_arm: Option<MissionAction>,
    /// The reason last announced for holding the start, while it holds.
    withheld: Option<StartWithheldReason>,
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
        self.fresh_status(now)
            .is_some_and(|status| status.ready_to_arm)
    }

    /// Whether the latest fresh vehicle status shows a launch under the
    /// configured [`LaunchDetection`]. `false` with no detection
    /// configured: the vehicle is then launched when the mission says so.
    #[must_use]
    pub fn launched(&self, now: MonotonicNanos) -> bool {
        let (Some(detection), Some(status)) =
            (self.config.launch_detection, self.fresh_status(now))
        else {
            return false;
        };
        detection.launched(&status)
    }

    /// Whether the latest fresh vehicle status says the flight
    /// controller's own position is valid.
    pub(super) fn fc_position_valid(&self, now: MonotonicNanos) -> bool {
        self.fresh_status(now)
            .is_some_and(|status| status.position_valid == Some(true))
    }

    /// The flight controller's height above its local origin in the
    /// latest fresh vehicle status, metres.
    pub(super) fn fc_height_m(&self, now: MonotonicNanos) -> Option<f64> {
        self.fresh_status(now)?
            .height_above_origin_m
            .filter(|height| height.is_finite())
    }

    /// The latest vehicle status no older than
    /// [`VEHICLE_STATUS_MAX_AGE_NS`] at `now`.
    fn fresh_status(&self, now: MonotonicNanos) -> Option<VehicleStatus> {
        self.vehicle.latest.filter(|status| {
            now.as_nanos().saturating_sub(status.acquired_at.as_nanos())
                <= VEHICLE_STATUS_MAX_AGE_NS
        })
    }

    /// The vehicle's lifecycle as the mission core reads it: ready while
    /// a fresh status says so, converging while the latest says it is not
    /// or has aged out, and unknown before any status.
    pub(super) fn vehicle_lifecycle(&self, now: MonotonicNanos) -> Option<VehicleLifecycleState> {
        self.vehicle.latest?;
        Some(if self.vehicle_ready(now) {
            VehicleLifecycleState::Ready
        } else {
            VehicleLifecycleState::Converging
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
        self.announce_start(Some(StartWithheldReason::VehicleNotReady), events);
        None
    }

    /// Announces why the start is held, once for each reason in a row: a
    /// changed reason is announced again, a changed uncertainty under the
    /// same reason is not. `None` clears it, silently: the start announces
    /// itself.
    pub(super) fn announce_start(
        &mut self,
        reason: Option<StartWithheldReason>,
        events: &mut Vec<MissionEvent>,
    ) {
        let changed = match (self.vehicle.withheld, reason) {
            (Some(held), Some(now)) => {
                core::mem::discriminant(&held) != core::mem::discriminant(&now)
            }
            (None, Some(_)) => true,
            (_, None) => false,
        };
        self.vehicle.withheld = reason;
        if let (true, Some(reason)) = (changed, reason) {
            events.push(MissionEvent::StartWithheld { reason });
        }
    }

    /// Why the start is held at `now`, before the arm request goes out:
    /// no solution to fly, or a vehicle that is not ready when the
    /// configuration waits for it.
    pub(super) fn start_withheld_reason(
        &self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
    ) -> Option<StartWithheldReason> {
        if solution.is_none() {
            return Some(self.solution_absence(now));
        }
        (self.config.arm_when_vehicle_ready && !self.vehicle_ready(now))
            .then_some(StartWithheldReason::VehicleNotReady)
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
