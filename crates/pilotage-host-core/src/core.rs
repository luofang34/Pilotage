//! The core and its calls.

use navigate_contract::{GeodeticPosition, MonotonicNanos, NavigationSolution};
use pilotage_mission::{WaypointPlan, WaypointPlanRecord};
use web_time::Instant;

use crate::config::HostCoreConfig;
use crate::error::HostCoreError;
use crate::output::{HostEvent, HostStatus, NextWake};

/// One vehicle's Pilotage host, stepped by the embedding host.
///
/// Every time argument is nanoseconds since `origin`, the instant given
/// to [`Self::new`], on the embedding host's clock. Times never go back.
#[derive(Debug)]
pub struct HostCore {
    config: HostCoreConfig,
    origin: Instant,
    last_now: MonotonicNanos,
    status: HostStatus,
    events: Vec<HostEvent>,
}

impl HostCore {
    /// A core for one vehicle, at time zero.
    ///
    /// # Errors
    ///
    /// [`HostCoreError::Adapter`] when the adapter does not accept the
    /// control-feel law or the link configuration.
    pub fn new(config: HostCoreConfig, origin: Instant) -> Result<Self, HostCoreError> {
        Ok(Self {
            config,
            origin,
            last_now: MonotonicNanos::from_nanos(0),
            status: HostStatus::default(),
            events: Vec::new(),
        })
    }

    /// Loads a waypoint plan as the mission to fly. The mission starts
    /// with the next tick: it arms the vehicle, climbs, and flies the
    /// plan.
    ///
    /// # Errors
    ///
    /// [`HostCoreError::MissionBuild`] when the plan does not build under
    /// the core's mission configuration.
    pub fn load_mission(
        &mut self,
        _plan: WaypointPlan,
    ) -> Result<WaypointPlanRecord, HostCoreError> {
        Err(HostCoreError::NotReady {
            operation: "load_mission",
        })
    }

    /// Runs everything that is due at `now` and returns when to call
    /// again.
    ///
    /// # Errors
    ///
    /// [`HostCoreError::TimeWentBack`] when `now` is earlier than the time
    /// of an earlier call.
    pub fn tick(&mut self, now: MonotonicNanos) -> Result<NextWake, HostCoreError> {
        self.advance_to(now)?;
        Err(HostCoreError::NotReady { operation: "tick" })
    }

    /// One datagram the flight controller sent on its telemetry link.
    pub fn on_downlink(&mut self, _datagram: &[u8], _now: MonotonicNanos) {}

    /// One datagram the flight controller sent on its command link.
    pub fn on_uplink_reply(&mut self, _datagram: &[u8], _now: MonotonicNanos) {}

    /// The MAVLink frames for the flight controller's command port since
    /// the last call, in send order.
    pub fn drain_uplink(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }

    /// One solution of the onboard navigator; the mission flies it when
    /// its ownship source is the onboard navigator.
    pub fn on_onboard_solution(&mut self, _solution: &NavigationSolution, _now: MonotonicNanos) {}

    /// The position the mission declares as its start: the mission
    /// anchor, where the embedding host places the vehicle.
    #[must_use]
    pub fn declared_start(&self) -> GeodeticPosition {
        self.config.mission.anchor
    }

    /// Where the core stands after its last call.
    #[must_use]
    pub fn status(&self) -> &HostStatus {
        &self.status
    }

    /// The events since the last call, in order.
    pub fn drain_events(&mut self) -> Vec<HostEvent> {
        core::mem::take(&mut self.events)
    }

    /// The instant all core times count from.
    #[must_use]
    pub fn origin(&self) -> Instant {
        self.origin
    }

    fn advance_to(&mut self, now: MonotonicNanos) -> Result<(), HostCoreError> {
        if now < self.last_now {
            return Err(HostCoreError::TimeWentBack {
                now_ns: now.as_nanos(),
                previous_ns: self.last_now.as_nanos(),
            });
        }
        self.last_now = now;
        Ok(())
    }
}
