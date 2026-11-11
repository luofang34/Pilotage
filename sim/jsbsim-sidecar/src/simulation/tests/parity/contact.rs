//! How far the readings of a vehicle on its gear may part between the
//! two engines.
//!
//! The comparison admits small differences in height and tilt between the
//! two engines. On the gear the contact forces change with those
//! differences, so a reading may part by what they cause. In the air
//! nothing is added to `FORCE_M_S2` and `RATE_RAD_S`. A difference counts
//! at most up to its own fixed limit (`POSITION_M`, `ATTITUDE_RAD`), so a
//! bound never grows with a difference that the comparison refuses. Each
//! term has one source:
//!
//! - Strut force `k·x + c·ẋ` for a compression `x`, with `k` and `c` from
//!   the model file: `FGLGear::ComputeVerticalStrutForce`.
//! - A contact that enters the ground during a physics step has its
//!   compression speed limited to `x/dt`: `FGLGear::GetBodyForces`
//!   (`maxCompressSpeed`). Its force is `(k + c/dt)·x` for that step, so
//!   its height sensitivity is `k + c/dt`. A contact that stays on the
//!   ground has the height sensitivity `k`.
//! - A contact at body position `r` changes height by `r·Δθ` for a tilt
//!   difference `Δθ`: the contact point position in
//!   `FGLGear::GetBodyForces`.
//! - The friction of a contact is at most its friction coefficient
//!   (`static_friction`, `dynamic_friction`, both below 1 in the test
//!   models) times its normal force: `FGLGear::ComputeJacobian` and
//!   `FGLGear::UpdateForces`. So the per-contact normal-force change,
//!   summed without sign, bounds the change on every axis.
//! - Mass and principal inertias: `inertia/mass-slugs` and
//!   `inertia/ixx-slugs_ft2` and so on, from `FGMassBalance`.
//! - The contact moments add to the body moment (`FGGroundReactions`
//!   sums `GetMoments` of each contact), and the body rates integrate the
//!   moment divided by the inertia: `FGAccelerations::CalculatePQRdot`.
//! - The strut damping opposes a rate difference `Δω` with the torque
//!   `-Σc·r·rᵀ·Δω`: the damping term of
//!   `FGLGear::ComputeVerticalStrutForce` at the contact speed `r × Δω`.
//!
//! The bounds:
//!
//! - Specific force, each axis: `Σ_contacts k_h·(|Δh| + |r|·|Δθ|) / m`
//!   added to `FORCE_M_S2`, with `k_h` the height sensitivity above.
//! - Body rate: a rate is integrated, so a difference that a contact step
//!   causes stays. Each step adds the angular impulse that the height and
//!   tilt differences can cause, divided by the smallest principal
//!   inertia: a common height difference gives `|Σk_h·r|·|Δh|`, and a tilt
//!   difference at most `λ_max(Σk_h·r·rᵀ)·|Δθ|`. The entry sensitivity
//!   holds for one physics step of a sample. The allowance decays by
//!   `exp(-dt·λ_min(Σc·r·rᵀ)/I)` on each step, the slowest decay that the
//!   damping torque gives.
//! - Contact compression: a contact on the ground in both engines may
//!   part in compression by at most `(FORCE_CAP_M_S2 - FORCE_M_S2)·m / Σk`,
//!   the difference at which the spring force alone takes the
//!   specific-force bound to its cap. A contact on the ground in one
//!   engine only fails on its own.
//!
//! The landing test compares the state, then the readings, then the
//! contact states, and checks the caps last. A model difference fails
//! one of these comparisons whatever their order; the order decides only
//! which one a failure names. A cap failure means that the admitted
//! differences no longer let the comparison see anything, so no model
//! difference is expected to fail there.

use std::path::Path;

use super::super::super::PHYSICS_DT_S;
use super::{ATTITUDE_RAD, FORCE_CAP_M_S2, FORCE_M_S2, POSITION_M, RATE_RAD_S, Simulation};
use crate::engine::{Engine, Property};
use crate::truth::Truth;

const KG_PER_SLUG: f64 = 14.593_902_937_206_364;
const KG_M2_PER_SLUG_FT2: f64 = 1.355_817_948_331_400_4;
const M_PER_FT: f64 = 0.3048;

/// One contact of the model file: its strut coefficients and its arm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Strut {
    spring_n_per_m: f64,
    damping_n_s_per_m: f64,
    /// Body x and y of the contact from the reference point, metres.
    x_m: f64,
    y_m: f64,
}

/// Reads every contact of `model.xml`, in file order, which is JSBSim's
/// contact unit order. The test models state their values in SI units.
pub(super) fn struts(aircraft_dir: &Path, model: &str) -> Vec<Strut> {
    let text = std::fs::read_to_string(aircraft_dir.join(format!("{model}.xml"))).unwrap();
    text.split("<contact ")
        .skip(1)
        .map(|contact| {
            let body = contact.split("</contact>").next().unwrap();
            assert!(body.contains("<location unit=\"M\">"), "{body}");
            let x = value(body, "<x>");
            let y = value(body, "<y>");
            Strut {
                spring_n_per_m: value(body, "<spring_coeff unit=\"N/M\">"),
                damping_n_s_per_m: value(body, "<damping_coeff unit=\"N/M/SEC\">"),
                x_m: x,
                y_m: y,
            }
        })
        .collect()
}

/// The number that follows `open` in `body`.
fn value(body: &str, open: &str) -> f64 {
    let start = body
        .find(open)
        .unwrap_or_else(|| panic!("a contact has no {open}"))
        + open.len();
    let end = body[start..].find('<').unwrap() + start;
    body[start..end].trim().parse().unwrap()
}

/// The contact properties of one engine.
struct Probe {
    struts: Vec<Strut>,
    on_ground: Vec<Property>,
    compression_ft: Vec<Property>,
    compression_speed_fps: Vec<Property>,
    was_on_ground: Vec<bool>,
    mass_slug: Property,
    inertia_slug_ft2: [Property; 3],
}

/// The sums over one engine's contacts on the ground of one strut
/// sensitivity `s` at the contact position `r = (x, y)`: `Σs`, `Σs·|r|`,
/// `Σs·r`, and `Σs·r·rᵀ`.
#[derive(Debug, Clone, Copy, Default)]
struct Moments {
    sum: f64,
    arm: f64,
    x: f64,
    y: f64,
    xx: f64,
    yy: f64,
    xy: f64,
}

impl Moments {
    fn add(&mut self, sensitivity: f64, strut: &Strut) {
        let (x, y) = (strut.x_m, strut.y_m);
        self.sum += sensitivity;
        self.arm += sensitivity * x.hypot(y);
        self.x += sensitivity * x;
        self.y += sensitivity * y;
        self.xx += sensitivity * x * x;
        self.yy += sensitivity * y * y;
        self.xy += sensitivity * x * y;
    }

    /// `|Σs·r|`: the torque of one common height or speed difference.
    fn first(&self) -> f64 {
        self.x.hypot(self.y)
    }

    /// The largest and the smallest eigenvalue of `Σs·r·rᵀ`: the torque
    /// of one tilt or rate difference, at most and at least.
    fn second(&self) -> (f64, f64) {
        let mean = 0.5 * (self.xx + self.yy);
        let spread = (0.5 * (self.xx - self.yy)).hypot(self.xy);
        (mean + spread, mean - spread)
    }
}

/// What one engine's contacts admit at one sample.
#[derive(Debug, Clone, Default)]
struct Sensitivity {
    /// Contact force per metre of height: `k + c/dt` for a contact that
    /// entered the ground in this step, `k` for the others, N/m.
    height: Moments,
    /// The strut spring of every contact on the ground, N/m.
    spring: Moments,
    /// The strut damping of every contact on the ground, N·s/m.
    damping: Moments,
    /// Whether each contact is on the ground.
    on_ground: Vec<bool>,
    /// The compression of each contact, metres.
    compression_m: Vec<f64>,
    mass_kg: f64,
    smallest_inertia_kg_m2: f64,
}

impl Probe {
    fn new(simulation: &mut Simulation, struts: Vec<Strut>) -> Self {
        let engine = &mut simulation.engine;
        let unit = |engine: &mut Engine, i: usize, name: &str| {
            engine
                .property(&format!("contact/unit[{i}]/{name}"))
                .unwrap()
        };
        let count = struts.len();
        let mut property = |name: &str| engine.property(name).unwrap();
        let inertia_slug_ft2 = [
            property("inertia/ixx-slugs_ft2"),
            property("inertia/iyy-slugs_ft2"),
            property("inertia/izz-slugs_ft2"),
        ];
        let mass_slug = property("inertia/mass-slugs");
        let on_ground: Vec<Property> = (0..count).map(|i| unit(engine, i, "WOW")).collect();
        // The vehicle starts standing on its gear: a contact on the ground
        // at the start did not enter the ground in the first step.
        let was_on_ground = on_ground.iter().map(|&p| engine.get(p) > 0.5).collect();
        Self {
            on_ground,
            compression_ft: (0..count)
                .map(|i| unit(engine, i, "compression-ft"))
                .collect(),
            compression_speed_fps: (0..count)
                .map(|i| unit(engine, i, "compression-velocity-fps"))
                .collect(),
            was_on_ground,
            mass_slug,
            inertia_slug_ft2,
            struts,
        }
    }

    /// The sensitivities at the current state. A contact entered the
    /// ground in this step when it was in the air at the last sample, or
    /// when JSBSim limited its compression speed to `x/dt`.
    fn sense(&mut self, simulation: &mut Simulation) -> Sensitivity {
        let engine = &mut simulation.engine;
        let mut sensitivity = Sensitivity {
            mass_kg: engine.get(self.mass_slug) * KG_PER_SLUG,
            smallest_inertia_kg_m2: self
                .inertia_slug_ft2
                .map(|p| engine.get(p) * KG_M2_PER_SLUG_FT2)
                .into_iter()
                .fold(f64::INFINITY, f64::min),
            ..Sensitivity::default()
        };
        for (i, strut) in self.struts.iter().enumerate() {
            let on_ground = engine.get(self.on_ground[i]) > 0.5;
            let compression_ft = engine.get(self.compression_ft[i]);
            let entered = on_ground
                && (!self.was_on_ground[i]
                    || speed_limited(compression_ft, engine.get(self.compression_speed_fps[i])));
            self.was_on_ground[i] = on_ground;
            sensitivity.on_ground.push(on_ground);
            sensitivity.compression_m.push(compression_ft * M_PER_FT);
            if !on_ground {
                continue;
            }
            sensitivity.spring.add(strut.spring_n_per_m, strut);
            sensitivity.damping.add(strut.damping_n_s_per_m, strut);
            let entry = if entered {
                strut.damping_n_s_per_m / PHYSICS_DT_S
            } else {
                0.0
            };
            sensitivity.height.add(strut.spring_n_per_m + entry, strut);
        }
        sensitivity
    }
}

/// Whether JSBSim limited the compression speed to `compression / dt`.
fn speed_limited(compression_ft: f64, speed_fps: f64) -> bool {
    let limit = compression_ft / PHYSICS_DT_S;
    compression_ft > 0.0 && (speed_fps.abs() - limit).abs() <= limit * 1e-12
}

/// The contacts of both engines over one flight.
pub(super) struct Contact {
    rust: Probe,
    cpp: Probe,
    rate_allowance_rad_s: f64,
}

impl Contact {
    pub(super) fn new(
        rust: &mut Simulation,
        rust_struts: Vec<Strut>,
        cpp: &mut Simulation,
        cpp_struts: Vec<Strut>,
    ) -> Self {
        Self {
            rust: Probe::new(rust, rust_struts),
            cpp: Probe::new(cpp, cpp_struts),
            rate_allowance_rad_s: 0.0,
        }
    }

    /// The bounds for the sample both engines reached after a step of
    /// `step_s` seconds.
    pub(super) fn bounds(
        &mut self,
        rust: &mut Simulation,
        cpp: &mut Simulation,
        r: &Truth,
        c: &Truth,
        step_s: f64,
    ) -> Bounds {
        // Each difference counts at most up to its own fixed limit, so a
        // bound never grows with a difference the comparison refuses.
        let height_m = (r.altitude_m - c.altitude_m).abs().min(POSITION_M);
        let tilt_rad = (r.attitude.roll - c.attitude.roll)
            .hypot(r.attitude.pitch - c.attitude.pitch)
            .min(ATTITUDE_RAD);
        let senses = [self.rust.sense(rust), self.cpp.sense(cpp)];
        let mut force = ForceBound {
            contact_m_s2: 0.0,
            k_per_s2: 0.0,
            height_m,
            tilt_rad,
        };
        let mut compression = compare_contacts(&senses[0], &senses[1]);
        let mut rate_step = 0.0f64;
        let mut retained = 0.0f64;
        for s in &senses {
            force.k_per_s2 = force.k_per_s2.max(s.height.sum / s.mass_kg);
            force.contact_m_s2 = force
                .contact_m_s2
                .max((s.height.sum * height_m + s.height.arm * tilt_rad) / s.mass_kg);
            if s.spring.sum > 0.0 {
                let limit = (FORCE_CAP_M_S2 - FORCE_M_S2) * s.mass_kg / s.spring.sum;
                compression.limit_m = compression.limit_m.min(limit);
            }
            // The entry sensitivity holds for the one physics step in
            // which a contact enters; the rest of the step sees the spring.
            let torque =
                |height: &Moments| height.first() * height_m + height.second().0 * tilt_rad;
            let impulse_n_m_s = PHYSICS_DT_S * torque(&s.height)
                + (step_s - PHYSICS_DT_S).max(0.0) * torque(&s.spring);
            rate_step = rate_step.max(impulse_n_m_s / s.smallest_inertia_kg_m2);
            let slowest = s.damping.second().1 / s.smallest_inertia_kg_m2;
            retained = retained.max((-step_s * slowest).exp());
        }
        self.rate_allowance_rad_s = self.rate_allowance_rad_s * retained + rate_step;
        Bounds {
            force,
            compression,
            rate_allowance_rad_s: self.rate_allowance_rad_s,
            horizontal_velocity_m_s: (r.velocity_ned[0] - c.velocity_ned[0])
                .hypot(r.velocity_ned[1] - c.velocity_ned[1]),
        }
    }
}

/// The contact states of the two engines: the first contact that is on
/// the ground in one engine only, and the largest compression difference
/// of the contacts on the ground in both. The caller sets the limit.
fn compare_contacts(a: &Sensitivity, b: &Sensitivity) -> CompressionBound {
    let mut bound = CompressionBound {
        difference_m: 0.0,
        limit_m: f64::INFINITY,
        one_sided: None,
    };
    for (i, (on_a, on_b)) in a.on_ground.iter().zip(&b.on_ground).enumerate() {
        if on_a != on_b {
            bound.one_sided = bound.one_sided.or(Some(i));
        } else if *on_a {
            let difference = (a.compression_m[i] - b.compression_m[i]).abs();
            bound.difference_m = bound.difference_m.max(difference);
        }
    }
    bound
}

/// The bounds of one sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Bounds {
    pub(super) force: ForceBound,
    pub(super) compression: CompressionBound,
    /// The body-rate difference that the contact steps so far admit.
    pub(super) rate_allowance_rad_s: f64,
    /// The horizontal velocity difference of the two truths, m/s. Both
    /// receivers add the same seeded noise, so it is also the difference
    /// of the two reported velocities.
    pub(super) horizontal_velocity_m_s: f64,
}

impl Bounds {
    /// A sample of a flight with no contact on the ground.
    pub(super) const AIRBORNE: Self = Self {
        force: ForceBound {
            contact_m_s2: 0.0,
            k_per_s2: 0.0,
            height_m: 0.0,
            tilt_rad: 0.0,
        },
        compression: CompressionBound {
            difference_m: 0.0,
            limit_m: f64::INFINITY,
            one_sided: None,
        },
        rate_allowance_rad_s: 0.0,
        horizontal_velocity_m_s: 0.0,
    };

    /// The widest the reported courses may part, in centidegrees, at a
    /// reported ground speed of `ground_speed_m_s`: five centidegrees
    /// plus the largest turn that the velocity difference can give the
    /// velocity, `asin(|Δv| / speed)`. When the difference is as large as
    /// the speed the course has no direction, and any two courses agree.
    /// The course is `atan2(ve, vn)` of the reported velocity
    /// (`Encoder::hil_gps`).
    pub(super) fn course_limit_cdeg(self, ground_speed_m_s: f64) -> f64 {
        if self.horizontal_velocity_m_s >= ground_speed_m_s {
            return f64::INFINITY;
        }
        5.0 + (self.horizontal_velocity_m_s / ground_speed_m_s)
            .asin()
            .to_degrees()
            * 100.0
    }

    pub(super) fn rate_limit(self) -> f64 {
        RATE_RAD_S + self.rate_allowance_rad_s
    }

    pub(super) fn describe_rate(self) -> String {
        format!(
            "bound {} = {RATE_RAD_S} + contact allowance {} rad/s",
            self.rate_limit(),
            self.rate_allowance_rad_s
        )
    }
}

/// The widest the two specific-force readings may part on one sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ForceBound {
    /// The specific-force change that the admitted height and tilt
    /// differences can cause at the contacts on the ground, m/s².
    pub(super) contact_m_s2: f64,
    /// The height sensitivity of the contacts on the ground, divided by
    /// the mass, per second².
    pub(super) k_per_s2: f64,
    /// The height difference between the engines, metres.
    pub(super) height_m: f64,
    /// The tilt difference between the engines, radians.
    pub(super) tilt_rad: f64,
}

impl ForceBound {
    pub(super) fn limit(self) -> f64 {
        FORCE_M_S2 + self.contact_m_s2
    }

    pub(super) fn describe(self) -> String {
        format!(
            "bound {} = {FORCE_M_S2} + contact {} m/s² (k {} /s², |Δh| {} m, |Δtilt| {} rad)",
            self.limit(),
            self.contact_m_s2,
            self.k_per_s2,
            self.height_m,
            self.tilt_rad
        )
    }
}

/// How far the compression of one contact may part between the engines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CompressionBound {
    /// The largest difference over the contacts on the ground in both
    /// engines, metres.
    pub(super) difference_m: f64,
    /// `(FORCE_CAP_M_S2 - FORCE_M_S2)·m / Σk`: the compression difference
    /// at which the spring force alone takes the specific-force bound to
    /// its cap. Infinite with no contact on the ground.
    pub(super) limit_m: f64,
    /// The first contact that is on the ground in one engine only.
    pub(super) one_sided: Option<usize>,
}

impl CompressionBound {
    pub(super) fn holds(self) -> bool {
        self.difference_m <= self.limit_m
    }
}
