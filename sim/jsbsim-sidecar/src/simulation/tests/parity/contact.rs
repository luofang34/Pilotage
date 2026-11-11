//! How far the readings of a vehicle on its gear may part between the
//! two engines.
//!
//! The comparison admits a height difference of up to `POSITION_M` and an
//! attitude difference of up to `ATTITUDE_RAD`. On the gear the contact
//! forces change with the height of each contact, so a reading may part
//! by what those admitted differences cause. In the air nothing is added
//! to `FORCE_M_S2` and `RATE_RAD_S`.
//!
//! The sensitivity comes from the model. JSBSim's strut force
//! (`FGLGear::ComputeVerticalStrutForce`) is `k·x + c·ẋ` for a compression
//! `x`. A contact that enters the ground during a step has its compression
//! speed limited to `x/dt` (`FGLGear::GetForces`), so its force is
//! `(k + c/dt)·x`: its sensitivity is `k + c/dt` to height and zero to
//! speed. A contact that stays on the ground has the sensitivity `k` to
//! height and `c` to speed.
//!
//! A contact's height difference between the engines is at most the
//! vehicle's height difference plus the contact's arm times the tilt
//! difference. Its speed difference is at most the vehicle's vertical
//! speed difference plus its arm times the body-rate difference.
//!
//! - Specific force: the sum over the contacts on the ground of each
//!   sensitivity times its difference, divided by the mass. The bound
//!   applies to each axis.
//! - Body rate: a rate is integrated, so a difference that one contact
//!   step causes stays. Each step on the ground adds the step time times
//!   the torque that the contact differences can cause, divided by the
//!   smallest principal moment of inertia.

use std::path::Path;

use super::super::super::PHYSICS_DT_S;
use super::{FORCE_M_S2, RATE_RAD_S, Simulation};
use crate::engine::{Engine, Property};
use crate::truth::Truth;

const KG_PER_SLUG: f64 = 14.593_902_937_206_364;
const KG_M2_PER_SLUG_FT2: f64 = 1.355_817_948_331_400_4;

/// One contact of the model file: its strut coefficients and its arm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Strut {
    spring_n_per_m: f64,
    damping_n_s_per_m: f64,
    /// Horizontal distance from the reference point, metres.
    arm_m: f64,
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
                arm_m: x.hypot(y),
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
/// sensitivity `s`: `Σs`, `Σs·arm`, and `Σs·arm²`.
#[derive(Debug, Clone, Copy, Default)]
struct Moments {
    sum: f64,
    arm: f64,
    arm2: f64,
}

impl Moments {
    fn add(&mut self, sensitivity: f64, arm_m: f64) {
        self.sum += sensitivity;
        self.arm += sensitivity * arm_m;
        self.arm2 += sensitivity * arm_m * arm_m;
    }
}

/// What one engine's contacts admit at one sample.
#[derive(Debug, Clone, Copy, Default)]
struct Sensitivity {
    /// Contact force per metre of height, N/m.
    height: Moments,
    /// Contact force per metre per second of compression speed, N·s/m.
    speed: Moments,
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
        Self {
            on_ground: (0..count).map(|i| unit(engine, i, "WOW")).collect(),
            compression_ft: (0..count)
                .map(|i| unit(engine, i, "compression-ft"))
                .collect(),
            compression_speed_fps: (0..count)
                .map(|i| unit(engine, i, "compression-velocity-fps"))
                .collect(),
            was_on_ground: vec![false; count],
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
            let entered = on_ground
                && (!self.was_on_ground[i]
                    || speed_limited(
                        engine.get(self.compression_ft[i]),
                        engine.get(self.compression_speed_fps[i]),
                    ));
            self.was_on_ground[i] = on_ground;
            if !on_ground {
                continue;
            }
            if entered {
                let stiffness = strut.spring_n_per_m + strut.damping_n_s_per_m / PHYSICS_DT_S;
                sensitivity.height.add(stiffness, strut.arm_m);
            } else {
                sensitivity.height.add(strut.spring_n_per_m, strut.arm_m);
                sensitivity.speed.add(strut.damping_n_s_per_m, strut.arm_m);
            }
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
        let height_m = (r.altitude_m - c.altitude_m).abs();
        let speed_m_s = (r.velocity_ned[2] - c.velocity_ned[2]).abs();
        let tilt_rad = (r.attitude.roll - c.attitude.roll)
            .abs()
            .max((r.attitude.pitch - c.attitude.pitch).abs());
        let rate_rad_s = (0..3)
            .map(|i| (r.rates_frd[i] - c.rates_frd[i]).abs())
            .fold(0.0, f64::max);
        let mut force = ForceBound {
            contact_n: 0.0,
            k_per_s2: 0.0,
            height_m,
            tilt_rad,
            speed_m_s,
            rate_rad_s,
        };
        let mut rate_step = 0.0f64;
        for s in [self.rust.sense(rust), self.cpp.sense(cpp)] {
            force.k_per_s2 = force.k_per_s2.max(s.height.sum / s.mass_kg);
            force.contact_n = force.contact_n.max(
                (s.height.sum * height_m
                    + s.height.arm * tilt_rad
                    + s.speed.sum * speed_m_s
                    + s.speed.arm * rate_rad_s)
                    / s.mass_kg,
            );
            let torque_n_m = s.height.arm * height_m
                + s.height.arm2 * tilt_rad
                + s.speed.arm * speed_m_s
                + s.speed.arm2 * rate_rad_s;
            rate_step = rate_step.max(step_s * torque_n_m / s.smallest_inertia_kg_m2);
        }
        self.rate_allowance_rad_s += rate_step;
        Bounds {
            force,
            rate_allowance_rad_s: self.rate_allowance_rad_s,
            horizontal_velocity_m_s: (r.velocity_ned[0] - c.velocity_ned[0])
                .hypot(r.velocity_ned[1] - c.velocity_ned[1]),
        }
    }
}

/// The bounds of one sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Bounds {
    pub(super) force: ForceBound,
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
            contact_n: 0.0,
            k_per_s2: 0.0,
            height_m: 0.0,
            tilt_rad: 0.0,
            speed_m_s: 0.0,
            rate_rad_s: 0.0,
        },
        rate_allowance_rad_s: 0.0,
        horizontal_velocity_m_s: 0.0,
    };

    /// The widest the reported courses may part, in centidegrees, at a
    /// reported ground speed of `ground_speed_m_s`: five centidegrees
    /// plus the course change that the velocity difference makes at
    /// that speed. A course of no speed is unbounded.
    pub(super) fn course_limit_cdeg(self, ground_speed_m_s: f64) -> f64 {
        5.0 + (self.horizontal_velocity_m_s / ground_speed_m_s)
            .atan()
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
    /// The specific-force change that the admitted differences can cause
    /// at the contacts on the ground, m/s².
    pub(super) contact_n: f64,
    /// The height sensitivity of the contacts on the ground, divided by
    /// the mass, per second².
    pub(super) k_per_s2: f64,
    /// The differences between the engines that cause it.
    pub(super) height_m: f64,
    pub(super) tilt_rad: f64,
    pub(super) speed_m_s: f64,
    pub(super) rate_rad_s: f64,
}

impl ForceBound {
    pub(super) fn limit(self) -> f64 {
        FORCE_M_S2 + self.contact_n
    }

    pub(super) fn describe(self) -> String {
        format!(
            "bound {} = {FORCE_M_S2} + contact {} m/s² (k {} /s², |Δh| {} m, |Δtilt| {} rad, |Δvd| {} m/s, |Δrate| {} rad/s)",
            self.limit(),
            self.contact_n,
            self.k_per_s2,
            self.height_m,
            self.tilt_rad,
            self.speed_m_s,
            self.rate_rad_s
        )
    }
}
