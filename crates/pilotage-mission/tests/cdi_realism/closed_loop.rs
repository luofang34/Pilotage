//! The closed-loop rig the CDI scenarios fly: a kinematic truth model
//! integrates the engine's own commanded intents at 20 Hz, plus an
//! optional lateral disturbance standing in for the wind that pushes a
//! vehicle off the course it is tracking.
//!
//! Nothing here reads a clock or randomizes: every step advances a
//! caller-owned `now` by a fixed interval, so a scenario's step counts
//! are reproducible numbers a regression can be measured against.

use aerocontext_core::{GeoPoint, NavDataCycle, NavDataSnapshot, NavPoint, NavPointKind};
use aerocontext_navdata::blob;
use chrono::NaiveDate;
use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::{LocalTangentPlane, NedOffset};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{
    MissionBuildError, MissionConfig, MissionEngine, MissionOutput, MissionPlanRecord,
    MissionState, NavGuidance, OwnshipSample, TruthRole, decode_snapshot,
};
use pilotage_protocol::{ControlAction, ControlIntent, ReferenceFrame};

/// The anchor every scenario flies from, matching the closed-loop tests
/// in `mission_engine.rs` so both suites read the same geometry.
pub const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};

/// Tick interval, nanoseconds: 20 Hz, the engine's own frame hint.
const STEP_NANOS: u64 = 50_000_000;

/// Tick interval in seconds, the truth model's integration step.
const STEP_S: f64 = 0.05;

/// The corner route's fixes in fly order. Five-letter, digit-free idents:
/// the route tokenizer reads a trailing digit as a procedure and a
/// published family prefix as an airway.
pub const CORNER_IDENTS: [&str; 3] = ["CORNA", "CORNB", "CORNC"];

/// The corner route as a string: three fixes flown in order, no airway.
pub const CORNER_ROUTE: &str = "CORNA CORNB CORNC";

/// NED offsets of the corner route's fixes from the anchor, meters.
///
/// `CORNA` and `CORNB` sit due east of the anchor, so the initial
/// direct-to leg runs along the extension of the first fixed leg and
/// arrives with no cross-track transient of its own — a scenario that
/// displaces the vehicle deliberately starts from a settled course.
/// `CORNC` lies due south of `CORNB`, making a 90° right turn at the
/// corner. Both fixed legs are 500 m, far outside the 100 m capture
/// radius, leaving room for a deviation to be built and nulled inside
/// one leg.
const CORNER_OFFSETS: [(f64, f64); 3] = [(0.0, 400.0), (0.0, 900.0), (-500.0, 900.0)];

/// The corner snapshot's cycle date, on the same 28-day AIRAC grid the
/// demo fixture uses.
const CORNER_EFFECTIVE_DATE: NaiveDate = match NaiveDate::from_ymd_opt(2026, 1, 22) {
    Some(date) => date,
    None => NaiveDate::MIN,
};

/// Planar truth: NED position and yaw integrated from the engine's own
/// BodyFrd commands (the exact inverse of the engine's NED-to-body
/// rotation), plus the lateral disturbance the scenario is applying.
struct Truth {
    ned: [f64; 3],
    ned_velocity: [f64; 3],
    yaw: f64,
    sequence: u32,
}

impl Truth {
    fn new() -> Self {
        Self {
            ned: [0.0; 3],
            ned_velocity: [0.0; 3],
            yaw: 0.0,
            sequence: 0,
        }
    }

    fn sample(&self, now: MonotonicNanos) -> OwnshipSample {
        OwnshipSample {
            ned: self.ned,
            ned_velocity: self.ned_velocity,
            yaw_rad: Some(self.yaw),
            role: TruthRole::SimulationTruth,
            acquired_at: now,
            sequence: self.sequence,
        }
    }

    fn integrate(&mut self, intent: &ControlIntent, disturbance_ned: [f64; 2]) {
        let ControlIntent::Velocity(v) = intent else {
            panic!("mission emits only velocity intents, got {intent:?}");
        };
        assert_eq!(v.frame, ReferenceFrame::BodyFrd);
        let (vx, vy, vz) = (f64::from(v.vx), f64::from(v.vy), f64::from(v.vz));
        let (sin, cos) = self.yaw.sin_cos();
        self.ned_velocity = [
            vx * cos - vy * sin + disturbance_ned[0],
            vx * sin + vy * cos + disturbance_ned[1],
            vz,
        ];
        for axis in 0..3 {
            self.ned[axis] += self.ned_velocity[axis] * STEP_S;
        }
        self.yaw += f64::from(v.yaw_rate) * STEP_S;
    }
}

/// One engine flown closed-loop against [`Truth`].
pub struct Rig {
    pub engine: MissionEngine,
    truth: Truth,
    now: MonotonicNanos,
    /// Horizontal NED velocity added to the truth each step, m/s: the
    /// disturbance a scenario uses to displace the vehicle from the
    /// course it is tracking. A rate the fusion innovation gate accepts
    /// keeps the displacement an observed one rather than a teleport the
    /// filter would reject outright.
    pub disturbance_ned: [f64; 2],
}

impl Rig {
    #[must_use]
    pub fn new(engine: MissionEngine) -> Self {
        Self {
            engine,
            truth: Truth::new(),
            now: MonotonicNanos::from_nanos(1_000_000_000),
            disturbance_ned: [0.0; 2],
        }
    }

    /// One closed-loop step: sample, tick, accept any arm, integrate any
    /// intent.
    pub fn step(&mut self) -> MissionOutput {
        self.now = MonotonicNanos::from_nanos(self.now.as_nanos() + STEP_NANOS);
        self.truth.sequence = self.truth.sequence.wrapping_add(1);
        self.engine
            .on_ownship(&self.truth.sample(self.now), self.now);
        let out = self.engine.tick(self.now);
        if let Some(action) = out.action {
            assert!(matches!(action.action, ControlAction::Arm));
            self.engine.on_action_result(action.action_id, true);
        }
        if let Some(intent) = out.intent.as_ref() {
            self.truth.integrate(intent, self.disturbance_ned);
        }
        out
    }

    /// Steps until `settled` accepts the published guidance, returning
    /// the step count it took. `None` means the budget ran out — a
    /// scenario asserts on that rather than reading a partial result.
    pub fn fly_until(
        &mut self,
        budget: usize,
        settled: impl Fn(&NavGuidance) -> bool,
    ) -> Option<usize> {
        for taken in 1..=budget {
            self.step();
            assert_ne!(
                self.engine.state(),
                MissionState::Complete,
                "the plan completed before the scenario's condition held"
            );
            if self.guidance().is_some_and(|g| settled(&g)) {
                return Some(taken);
            }
        }
        None
    }

    /// The guidance a display would be showing right now.
    #[must_use]
    pub fn guidance(&self) -> Option<NavGuidance> {
        self.engine.nav_guidance()
    }

    /// The published guidance, or a failure naming what was expected of
    /// it: a scenario asserting on deviation geometry has no meaningful
    /// fallback when the executor is flying nothing.
    #[must_use]
    pub fn expect_guidance(&self, what: &str) -> NavGuidance {
        self.guidance()
            .unwrap_or_else(|| panic!("guidance must be published while {what}"))
    }

    /// The cross-track deviation a display would be showing, meters.
    #[must_use]
    pub fn expect_deviation(&self, what: &str) -> f64 {
        self.expect_guidance(what)
            .lateral_deviation_m
            .unwrap_or_else(|| panic!("a fixed leg reports cross-track deviation while {what}"))
    }
}

/// The NED unit vector right of `course_rad`, scaled by `speed_mps`.
///
/// A course is a bearing clockwise from north, so its along-track unit is
/// `(cos, sin)` in (north, east) and right of it is that quarter turn
/// clockwise. Deriving the push direction from the published course is
/// what makes the sign assertion independent: geodesy's "positive right
/// of track" must agree with the bearing convention, not merely with
/// itself.
#[must_use]
pub fn right_of_course_ned(course_rad: f64, speed_mps: f64) -> [f64; 2] {
    let (sin, cos) = course_rad.sin_cos();
    [-sin * speed_mps, cos * speed_mps]
}

/// The demo-fixture engine: the shipped three-fix route, so a scenario
/// can pin the geometry an operator actually flies.
#[must_use]
pub fn fixture_engine() -> MissionEngine {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    engine_over(&blob, fixture::DEMO_ROUTE)
}

/// The corner-route engine: [`CORNER_OFFSETS`] packed through the same
/// blob container published data travels.
#[must_use]
pub fn corner_engine() -> MissionEngine {
    let (engine, _record) = try_engine_over_offsets(&CORNER_IDENTS, &CORNER_OFFSETS, CORNER_ROUTE)
        .expect("the corner mission builds");
    engine
}

/// Packs arbitrary NED fix offsets through the blob container published
/// data travels and attempts the mission build, surfacing the typed
/// refusal a route the sequencer cannot activate earns.
pub fn try_engine_over_offsets(
    idents: &[&str],
    offsets: &[(f64, f64)],
    route: &str,
) -> Result<(MissionEngine, MissionPlanRecord), MissionBuildError> {
    let plane = LocalTangentPlane::new(anchor_position()).expect("the anchor is plausible");
    let points = idents
        .iter()
        .zip(offsets.iter().copied())
        .map(|(ident, (north_m, east_m))| {
            let position = plane.from_ned(&NedOffset::new(north_m, east_m, 0.0));
            NavPoint::new(
                *ident,
                NavPointKind::Waypoint,
                GeoPoint {
                    lat: position.latitude_rad.to_degrees(),
                    lon: position.longitude_rad.to_degrees(),
                },
            )
        })
        .collect();
    let cycle = NavDataCycle::faa_nasr(CORNER_EFFECTIVE_DATE).expect("the cycle is valid");
    let snapshot = NavDataSnapshot::new(cycle, points);
    let blob = blob::encode(&snapshot).expect("the snapshot encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("the blob decodes");
    let config = MissionConfig::new(route.to_owned(), anchor_position(), ClockDomainId::new(7));
    MissionEngine::new(&snapshot, provenance, config)
}

/// The anchor as the engine sees it: radians and meters.
fn anchor_position() -> GeodeticPosition {
    GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    )
}

/// Builds an engine over a packed snapshot and a route, on the documented
/// mission defaults.
fn engine_over(packed: &[u8], route: &str) -> MissionEngine {
    let (snapshot, provenance) = decode_snapshot(packed, true).expect("the blob decodes");
    let config = MissionConfig::new(route.to_owned(), anchor_position(), ClockDomainId::new(7));
    let (engine, _record) =
        MissionEngine::new(&snapshot, provenance, config).expect("the mission builds");
    engine
}
