//! Mission configuration with documented defaults.

use navigate_contract::{ClockDomainId, DurationNanos, GeodeticPosition};
use pilotage_mission_core::ExecutionTarget;
use pilotage_protocol::VehicleClass;

use crate::error::MissionBuildError;
use crate::holding_pattern::HoldingTurn;

/// Where the engine's ownship position comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OwnshipSource {
    /// A simulator's truth oracle. The engine synthesizes a GNSS-class fix
    /// from each sample and fuses it. No vehicle has this source, so a
    /// mission for [`ExecutionTarget::RealVehicle`] refuses it at build.
    SimulationTruth,
    /// The vehicle's own estimate lane, as its flight controller exports
    /// it under the operational-estimate role. The engine flies the latest
    /// fresh sample as a pass-through solution, declares it
    /// estimator-derived, and runs no fusion. The solution never grades
    /// better than degraded. This is the default.
    VehicleEstimate {
        /// 1-sigma position uncertainty the solution declares per NED
        /// axis, meters.
        sigma_m: f64,
        /// 1-sigma velocity uncertainty the solution declares per NED
        /// axis, meters per second.
        velocity_sigma_mps: f64,
        /// Age past which the latest sample yields no solution.
        max_age: DurationNanos,
    },
    /// The onboard navigator's own solution, as the host hands it to
    /// [`crate::MissionEngine::on_onboard_solution`]. The engine flies the
    /// latest solution with the covariance, integrity, and composition
    /// the navigator gave it, and runs no fusion.
    OnboardNavigation {
        /// Age past which the latest solution yields no solution.
        max_age: DurationNanos,
        /// 1-sigma horizontal uncertainty above which the latest solution
        /// is beyond its limits, meters.
        max_horizontal_1sigma_m: f64,
        /// What a fresh solution beyond its limits (graded unusable, or
        /// with a horizontal uncertainty above the limit) does.
        beyond_limits: BeyondLimits,
    },
}

/// What the engine does with a fresh onboard solution beyond its limits:
/// graded unusable, or with a horizontal uncertainty above the limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BeyondLimits {
    /// The solution is not flown: the vehicle holds, and the start waits.
    Hold,
    /// In flight the solution is still flown: the plan goes on by dead
    /// reckoning, waypoints sequence on the solution's own position, and
    /// the engine reports the dead reckoning with its uncertainty. A
    /// navigator with no position aid (an inertial and barometric one
    /// without satellites) grows its uncertainty for the whole flight,
    /// and holding would end the flight where the uncertainty crossed a
    /// line. The start still waits for a solution within the limits, and
    /// a solution that is stale, absent, or of an unknown uncertainty is
    /// never flown.
    DeadReckon {
        /// The 1-sigma horizontal uncertainty past which the vehicle holds
        /// all the same, meters; `None` for no ceiling.
        ceiling_1sigma_m: Option<f64>,
    },
}

impl OwnshipSource {
    /// The vehicle-estimate source with its documented defaults: 3 m
    /// position, 0.5 m/s velocity, one second of age.
    #[must_use]
    pub const fn vehicle_estimate() -> Self {
        Self::VehicleEstimate {
            sigma_m: 3.0,
            velocity_sigma_mps: 0.5,
            max_age: DurationNanos::from_millis(1_000),
        }
    }

    /// The onboard-navigation source with its documented defaults: one
    /// second of age and 50 m of horizontal uncertainty. The age equals
    /// the bound guidance applies to every solution; the uncertainty
    /// equals the vertical bound guidance applies to an
    /// altitude-constrained leg.
    #[must_use]
    pub const fn onboard_navigation() -> Self {
        Self::OnboardNavigation {
            max_age: DurationNanos::from_millis(1_000),
            max_horizontal_1sigma_m: 50.0,
            beyond_limits: BeyondLimits::Hold,
        }
    }

    /// The onboard-navigation source for flight without satellite
    /// navigation: the limits of [`Self::onboard_navigation`], past which
    /// the plan goes on by dead reckoning with no ceiling
    /// ([`BeyondLimits::DeadReckon`]).
    #[must_use]
    pub const fn onboard_navigation_gnss_denied() -> Self {
        Self::OnboardNavigation {
            max_age: DurationNanos::from_millis(1_000),
            max_horizontal_1sigma_m: 50.0,
            beyond_limits: BeyondLimits::DeadReckon {
                ceiling_1sigma_m: None,
            },
        }
    }
}

/// The holding pattern a vehicle that cannot stop flies at the last
/// waypoint when its plan completes and it has no further instruction; see
/// [`crate::holding_pattern`]. Each value left unset takes its default.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TerminalHoldingPattern {
    /// The holding altitude on the WGS84 ellipsoid, metres. Default: the
    /// altitude of the last waypoint.
    pub altitude_m: Option<f64>,
    /// The direction of the turns. Default: right turns.
    pub turn: HoldingTurn,
    /// The inbound leg time, seconds. Default:
    /// [`crate::holding_pattern::standard_leg_time_s`] at the holding
    /// altitude.
    pub leg_time_s: Option<f64>,
}

/// Ceilings every emitted intent is clamped within.
///
/// The defaults mirror the reference adapters' advertised
/// `vehicle.motion` limits; the host MAY tighten them from the actually
/// advertised capabilities before constructing the engine. The engine
/// never emits beyond them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionLimits {
    /// Ceiling on the commanded horizontal speed, m/s. Default 2.5.
    /// An over-ceiling horizontal vector is scaled, never clipped per
    /// axis, so the cap cannot rotate the commanded direction.
    pub max_horizontal_mps: f64,
    /// Ceiling on the magnitude of the commanded vertical rate, m/s.
    /// Default 1.0.
    pub max_vertical_mps: f64,
    /// Ceiling on the magnitude of the commanded yaw rate, rad/s.
    /// Default 0.8.
    pub max_yaw_rate_rps: f64,
}

impl Default for MissionLimits {
    fn default() -> Self {
        Self {
            max_horizontal_mps: 2.5,
            max_vertical_mps: 1.0,
            max_yaw_rate_rps: 0.8,
        }
    }
}

/// Configuration of a [`crate::MissionEngine`].
///
/// The engine is sans-IO: `clock` names the single clock domain every
/// `now`, every sample stamp, and every fusion judgment lives on; the
/// engine itself never reads a clock.
#[derive(Debug, Clone, PartialEq)]
pub struct MissionConfig {
    /// Route string expanded against the navdata snapshot at build. A
    /// survey mission carries no route and leaves it empty.
    pub route: String,
    /// Mission anchor in radians/meters ([`GeodeticPosition`]). The host
    /// converts from the snapshot's degrees exactly once, at plan build
    /// (ADR-0030); nothing downstream ever sees degrees.
    pub anchor: GeodeticPosition,
    /// Cruise height above the anchor altitude, meters. Default 15.0.
    /// `0.0` disables climb behavior. The mission starts enroute guidance
    /// after the arm receipt.
    pub cruise_height_m: f64,
    /// Commanded climb rate during the climb phase, m/s. Default 0.8.
    pub climb_rate_mps: f64,
    /// Along-track cruise speed asked of guidance, m/s. Default 2.0.
    pub cruise_mps: f64,
    /// Intent ceilings; see [`MissionLimits`] for the defaults.
    pub limits: MissionLimits,
    /// Yaw alignment gain, inverse seconds: radians of heading error to
    /// rad/s of commanded yaw rate. Default 1.0.
    pub yaw_gain_per_s: f64,
    /// 1-sigma of the synthesized GNSS position fix per NED axis,
    /// meters. Default 1.5.
    pub gnss_sigma_m: f64,
    /// Hint to the host for how often to call [`crate::MissionEngine::tick`]
    /// and frame the result. Default 50 ms — well inside the session
    /// holder-silence watchdog. The engine itself never schedules.
    pub frame_interval: DurationNanos,
    /// The clock domain all mission time lives on.
    pub clock: ClockDomainId,
    /// Where ownship position comes from. Default
    /// [`OwnshipSource::vehicle_estimate`].
    pub ownship_source: OwnshipSource,
    /// The execution target the mission document is admitted for.
    /// Default [`ExecutionTarget::Simulator`]; a physical vehicle needs
    /// [`ExecutionTarget::RealVehicle`].
    pub execution_target: ExecutionTarget,
    /// The identities of the run's tasks in plan order, each survey line then the way
    /// home, which the core's phases take. Empty, the phases take the tasks' names.
    pub task_ids: Vec<String>,
    /// How long a host waits for the vehicle to confirm an arm before it
    /// answers the arm as refused. Default
    /// [`crate::policy::ARM_CONFIRM_TIMEOUT_NS`].
    pub arm_confirm_timeout: DurationNanos,
    /// The class of the vehicle the mission flies. It sets how a waypoint
    /// plan ends: a vehicle that can stop flies on to the last waypoint
    /// and is then commanded to zero velocity; a
    /// [`VehicleClass::FixedWing`], which cannot stop, completes the plan
    /// when the last waypoint sequences and then flies
    /// [`Self::terminal_holding_pattern`]. Default
    /// [`VehicleClass::Multicopter`].
    pub vehicle_class: VehicleClass,
    /// The time at cruise speed that sizes the capture radius of a
    /// waypoint plan that sets none, seconds. Default 2.0.
    pub capture_time_s: f64,
    /// The holding pattern after a plan for a vehicle that cannot stop.
    pub terminal_holding_pattern: TerminalHoldingPattern,
    /// The bank angle the mission plans its turns at, degrees. With the
    /// speed over the ground it sizes the turn ahead of a fly-by waypoint.
    /// It also limits the bank of the holding pattern after the plan.
    ///
    /// The host sets it below the bank the flight controller turns at on a
    /// route. A turn planned at the airframe's bank limit leaves the
    /// aircraft no bank to spare: the time it takes to roll in, and every
    /// gust or shift of wind in the turn, then carry it outside the planned
    /// path, and it has no authority left to come back. The margin below
    /// the limit is that authority. Default
    /// [`crate::DEFAULT_ROUTE_BANK_DEG`].
    pub route_bank_deg: f64,
    /// Whether the arm request waits for a vehicle status that says the
    /// vehicle is ready to arm; see [`crate::VehicleStatus`]. A host that
    /// feeds vehicle status sets it, so no arm goes to a flight controller
    /// still starting up. Default false: a host that feeds no status gets
    /// the arm request at once.
    pub arm_when_vehicle_ready: bool,
    /// When the vehicle counts as launched by its flight controller's own
    /// height or airspeed, with no position: a fixed wing launched with no
    /// solution fit to fly then climbs on its heading instead of waiting
    /// without a command. Default none.
    pub launch_detection: Option<crate::LaunchDetection>,
    /// The operator's statement of the start, for a navigator with no
    /// satellite fix to start from. The engine does not fly it or read it:
    /// [`crate::MissionEngine::declared_start`] hands it on to the
    /// navigator as stated. Default none.
    pub declared_start: Option<crate::DeclaredStart>,
    /// The height above the WGS84 ellipsoid of the flight controller's
    /// local origin, metres: the datum its reported heights stand on and
    /// that turns the altitudes the engine commands into its frame. With
    /// it, a vehicle that holds its altitude itself captures the climb on
    /// the altitude its flight controller flies. Default none.
    pub fc_origin_height_m: Option<f64>,
}

impl MissionConfig {
    /// Whether the vehicle can stop at a point and hold there.
    #[must_use]
    pub fn can_stop(&self) -> bool {
        self.vehicle_class != VehicleClass::FixedWing
    }

    /// Refuses an ownship source the execution target cannot supply.
    ///
    /// # Errors
    ///
    /// [`MissionBuildError::TruthOwnshipOnRealVehicle`] for
    /// [`OwnshipSource::SimulationTruth`] with
    /// [`ExecutionTarget::RealVehicle`]: a real vehicle has no truth
    /// oracle, so the mission would wait in flight for a solution that
    /// never comes.
    /// Refuses a declared start no navigator can take.
    ///
    /// # Errors
    ///
    /// [`MissionBuildError::DeclaredStart`] naming the refused part.
    pub(crate) fn check_declared_start(&self) -> Result<(), MissionBuildError> {
        self.declared_start
            .as_ref()
            .map_or(Ok(()), crate::DeclaredStart::validate)
            .map_err(|source| MissionBuildError::DeclaredStart { source })
    }

    pub(crate) fn check_ownship_source(&self) -> Result<(), MissionBuildError> {
        match (self.ownship_source, self.execution_target) {
            (OwnshipSource::SimulationTruth, ExecutionTarget::RealVehicle) => {
                Err(MissionBuildError::TruthOwnshipOnRealVehicle)
            }
            _ => Ok(()),
        }
    }

    /// A config over the documented defaults; the route, anchor, and
    /// clock domain have no meaningful default and are always the
    /// caller's.
    #[must_use]
    pub fn new(route: String, anchor: GeodeticPosition, clock: ClockDomainId) -> Self {
        Self {
            route,
            anchor,
            cruise_height_m: 15.0,
            climb_rate_mps: 0.8,
            cruise_mps: 2.0,
            limits: MissionLimits::default(),
            yaw_gain_per_s: 1.0,
            gnss_sigma_m: 1.5,
            frame_interval: DurationNanos::from_millis(50),
            clock,
            ownship_source: OwnshipSource::vehicle_estimate(),
            execution_target: ExecutionTarget::Simulator,
            task_ids: Vec::new(),
            arm_confirm_timeout: DurationNanos::from_nanos(crate::policy::ARM_CONFIRM_TIMEOUT_NS),
            vehicle_class: VehicleClass::Multicopter,
            capture_time_s: 2.0,
            terminal_holding_pattern: TerminalHoldingPattern::default(),
            route_bank_deg: crate::DEFAULT_ROUTE_BANK_DEG,
            arm_when_vehicle_ready: false,
            launch_detection: None,
            declared_start: None,
            fc_origin_height_m: None,
        }
    }
}
