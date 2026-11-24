//! A fixed wing flown closed-loop on the engine's intents: it turns to
//! the heading or course demanded at a limited rate, flies the airspeed
//! demanded, and climbs or descends to the altitude or at the rate
//! demanded.

use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::NedOffset;
use pilotage_protocol::{ControlAction, ControlIntent, VehicleClass};

use crate::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionEvent, MissionOutput, MissionState,
    OwnshipSample, OwnshipSource, PlannedWaypoint, TruthRole, WaypointPlan,
};

pub const CLOCK: ClockDomainId = ClockDomainId::new(7);
pub const STEP_NANOS: u64 = 50_000_000;
pub const STEP_S: f64 = 0.05;
pub const ALTITUDE_M: f64 = 2_000.0;
pub const CRUISE_MPS: f64 = 55.0;
pub const CEILING_MPS: f64 = 65.0;
/// The turn rate of the aircraft at its route bank, rad/s.
const TURN_RATE_RPS: f64 = 0.07;
/// The plan, as anchor-plane north and east offsets, metres: north along
/// the meridian of the anchor, then east.
pub const PLAN: [[f64; 2]; 3] = [[2_000.0, 0.0], [6_000.0, 0.0], [6_000.0, 4_000.0]];

/// Which source the engine flies on.
#[derive(Clone, Copy, PartialEq)]
pub enum Feed {
    /// The simulation truth: the engine steers by velocity.
    Truth,
    /// The onboard navigator: the engine steers by heading and altitude.
    Navigator,
}

pub fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

pub fn config(feed: Feed) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = match feed {
        Feed::Truth => OwnshipSource::SimulationTruth,
        Feed::Navigator => OwnshipSource::onboard_navigation(),
    };
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = CRUISE_MPS;
    config.limits.max_horizontal_mps = CEILING_MPS;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = 22.0;
    config
}

/// The waypoint at `north` and `east` of the anchor, metres.
pub fn waypoint([north, east]: [f64; 2]) -> PlannedWaypoint {
    let plane = crate::body::tangent_plane(anchor()).expect("the anchor is plausible");
    let position = plane.from_ned(&NedOffset::new(north, east, 0.0));
    PlannedWaypoint::new(
        position.latitude_rad.to_degrees(),
        position.longitude_rad.to_degrees(),
        ALTITUDE_M,
    )
}

pub fn plan() -> WaypointPlan {
    WaypointPlan::new(PLAN.into_iter().map(waypoint).collect())
}

/// The engine with the aircraft it flies.
pub struct Rig {
    pub engine: MissionEngine,
    feed: Feed,
    /// The position in the anchor plane, north, east, down, metres.
    pub ned: [f64; 3],
    pub heading_rad: f64,
    pub airspeed_mps: f64,
    climb_mps: f64,
    now: MonotonicNanos,
    sequence: u32,
    /// Whether the source gives the engine a position.
    pub position_known: bool,
    /// Each event of each tick, in order.
    pub events: Vec<MissionEvent>,
}

impl Rig {
    /// A rig 1000 m south of the first waypoint, northbound at cruise.
    pub fn new(feed: Feed) -> Self {
        Self::with(feed, config(feed), &plan(), 0.0)
    }

    /// A rig that flies `plan` under `config`, northbound at cruise from
    /// 1000 m north of the anchor and `east_m` east of it.
    pub fn with(feed: Feed, config: MissionConfig, plan: &WaypointPlan, east_m: f64) -> Self {
        let (engine, _) = MissionEngine::from_waypoints(plan, config).expect("builds");
        Self {
            engine,
            feed,
            ned: [1_000.0, east_m, 0.0],
            heading_rad: 0.0,
            airspeed_mps: CRUISE_MPS,
            climb_mps: 0.0,
            now: MonotonicNanos::from_nanos(1_000_000_000),
            sequence: 0,
            position_known: true,
            events: Vec::new(),
        }
    }

    pub fn altitude_m(&self) -> f64 {
        ALTITUDE_M - self.ned[2]
    }

    fn velocity(&self) -> [f64; 3] {
        let (sin, cos) = self.heading_rad.sin_cos();
        [
            self.airspeed_mps * cos,
            self.airspeed_mps * sin,
            -self.climb_mps,
        ]
    }

    /// One tick: the source's sample, the engine's tick, the arm's
    /// answer, and the aircraft's flight of the intent.
    pub fn step(&mut self) -> MissionOutput {
        self.now = MonotonicNanos::from_nanos(self.now.as_nanos() + STEP_NANOS);
        self.sequence = self.sequence.wrapping_add(1);
        self.feed_source();
        let output = self.engine.tick(self.now);
        if let Some(action) = output.action {
            assert!(matches!(action.action, ControlAction::Arm));
            self.engine.on_action_result(action.action_id, true);
        }
        self.events.extend(output.events.iter().cloned());
        self.fly(output.intent.as_ref());
        output
    }

    fn feed_source(&mut self) {
        if !self.position_known {
            return;
        }
        match self.feed {
            Feed::Truth => {
                let sample = OwnshipSample {
                    ned: self.ned,
                    ned_velocity: self.velocity(),
                    yaw_rad: Some(self.heading_rad),
                    role: TruthRole::SimulationTruth,
                    acquired_at: self.now,
                    sequence: self.sequence,
                };
                self.engine.on_ownship(&sample, self.now);
            }
            Feed::Navigator => {
                let solution = self.solution();
                self.engine.on_onboard_solution(&solution, self.now);
            }
        }
    }

    /// Steps until `done` or for `ticks`, and gives the last output.
    pub fn step_until(
        &mut self,
        ticks: usize,
        done: impl Fn(&Self, &MissionOutput) -> bool,
    ) -> MissionOutput {
        let mut output = self.step();
        for _ in 1..ticks {
            if done(self, &output) {
                break;
            }
            output = self.step();
        }
        output
    }

    /// Steps until the route is flown.
    pub fn fly_to_route(&mut self) {
        let output = self.step_until(400, |_, output| output.state == MissionState::Enroute);
        assert_eq!(output.state, MissionState::Enroute, "the route is flown");
    }

    fn fly(&mut self, intent: Option<&ControlIntent>) {
        let mut wanted = self.heading_rad;
        match intent {
            Some(ControlIntent::HeadingAltitude(demand)) => {
                wanted = f64::from(demand.heading_rad);
                let error_m = f64::from(demand.altitude_m) - self.altitude_m();
                self.climb_mps = (error_m / 5.0).clamp(-5.0, 5.0);
                if let Some(airspeed) = demand.airspeed_mps {
                    self.airspeed_mps = f64::from(airspeed);
                }
            }
            Some(ControlIntent::Velocity(velocity)) => {
                let (sin, cos) = self.heading_rad.sin_cos();
                let (vx, vy) = (f64::from(velocity.vx), f64::from(velocity.vy));
                let (north, east) = (vx * cos - vy * sin, vx * sin + vy * cos);
                if north.hypot(east) > 0.0 {
                    wanted = east.atan2(north);
                    self.airspeed_mps = north.hypot(east);
                }
                self.climb_mps = -f64::from(velocity.vz);
            }
            _ => {}
        }
        let error = (wanted - self.heading_rad)
            .sin()
            .atan2((wanted - self.heading_rad).cos());
        self.heading_rad += (error / 3.0).clamp(-TURN_RATE_RPS, TURN_RATE_RPS) * STEP_S;
        let velocity = self.velocity();
        for (position, rate) in self.ned.iter_mut().zip(velocity) {
            *position += rate * STEP_S;
        }
    }

    /// The onboard navigator's solution of the aircraft.
    fn solution(&self) -> NavigationSolution {
        const SIGMA_M: f64 = 5.0;
        let plane = crate::body::tangent_plane(anchor()).expect("the anchor is plausible");
        let position = plane.from_ned(&NedOffset::new(self.ned[0], self.ned[1], self.ned[2]));
        let velocity = self.velocity();
        let composition = SourceComposition::of(SensorClass::VisualLandmark);
        let variance = SIGMA_M * SIGMA_M;
        let half = self.heading_rad / 2.0;
        NavigationSolution::new(
            SolutionStamp::new(
                SourceEpoch::new(0),
                WrappingSequence::new(self.sequence),
                self.now,
                CLOCK,
            ),
            position,
            NedVelocity::new(velocity[0], velocity[1], velocity[2]),
            SymmetricCov3::from_diagonal(variance, variance, variance),
            SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
            IntegrityAssessment::new(
                SolutionQuality::Good,
                composition,
                Redundancy::None,
                SIGMA_M,
                SIGMA_M,
                FaultDetection::Unavailable,
            ),
            composition,
            Ellipsoid::WGS84,
        )
        .with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
    }
}
