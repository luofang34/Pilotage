//! The frames that a controller reads must agree during a landing.

use pilotage_world::{MagneticEpoch, UniformTerrain, World};

use super::super::{message_id, vehicle};
use super::contact::{Bounds, Contact, struts};
use super::{
    Aircraft, Encoder, EngineKind, FORCE_CAP_M_S2, GROUND_M, ModelPaths, N51, RATE_CAP_RAD_S,
    Simulation, Start, assert_same_state, close, controls,
};
use crate::hil::{HIL_GPS, HIL_SENSOR};

fn f32_at(frame: &[u8], at: usize) -> f64 {
    f64::from(f32::from_le_bytes(
        frame[10 + at..14 + at].try_into().unwrap(),
    ))
}

fn i32_at(frame: &[u8], at: usize) -> i64 {
    i64::from(i32::from_le_bytes(
        frame[10 + at..14 + at].try_into().unwrap(),
    ))
}

/// The horizontal speed of an `HIL_GPS` frame from its north and east
/// velocities, before the encoder rounds the speed to whole cm/s.
fn speed_of(frame: &[u8]) -> f64 {
    let cm_s = |at: usize| {
        f64::from(i16::from_le_bytes(
            frame[10 + at..12 + at].try_into().unwrap(),
        ))
    };
    cm_s(26).hypot(cm_s(28)) / 100.0
}

/// Two `HIL_GPS` frames carry the same fix, within the bounds.
fn assert_gps(at: &str, r: &[u8], c: &[u8], bounds: Bounds) {
    for offset in [8, 12, 16] {
        assert!(
            (i32_at(r, offset) - i32_at(c, offset)).abs() <= 1,
            "{at}: HIL_GPS at {offset}"
        );
    }
    assert_eq!(&r[30..34], &c[30..34], "{at}: GNSS accuracy");
    for offset in [26, 28, 30] {
        let value =
            |frame: &[u8]| i16::from_le_bytes(frame[10 + offset..12 + offset].try_into().unwrap());
        assert!(
            (i32::from(value(r)) - i32::from(value(c))).abs() <= 1,
            "{at}: HIL_GPS velocity at {offset}"
        );
    }
    let u16_at = |frame: &[u8], at: usize| {
        i32::from(u16::from_le_bytes(
            frame[10 + at..12 + at].try_into().unwrap(),
        ))
    };
    assert!(
        (u16_at(r, 24) - u16_at(c, 24)).abs() <= 1,
        "{at}: GNSS speed"
    );
    let course = (u16_at(r, 32) - u16_at(c, 32) + 18_000).rem_euclid(36_000) - 18_000;
    let speed_m_s = speed_of(r);
    let limit = bounds.course_limit_cdeg(speed_m_s);
    assert!(
        f64::from(course.abs()) <= limit,
        "{at}: GNSS course: Rust {} cdeg, C++ {} cdeg, bound {limit} cdeg at {speed_m_s} m/s, |Δv| {} m/s",
        u16_at(r, 32),
        u16_at(c, 32),
        bounds.horizontal_velocity_m_s
    );
    assert_eq!(
        &r[44..r.len() - 2],
        &c[44..c.len() - 2],
        "{at}: GNSS fix fields"
    );
}

fn assert_frames(at: &str, rust: &[Vec<u8>], cpp: &[Vec<u8>], bounds: Bounds) {
    assert_eq!(rust.len(), cpp.len(), "{at}: frame count");
    for (r, c) in rust.iter().zip(cpp) {
        assert_eq!(message_id(r), message_id(c));
        if message_id(r) != 0 {
            assert_eq!(&r[10..18], &c[10..18], "{at}: time");
        }
        if message_id(r) == HIL_SENSOR.0 {
            // Acceleration, rate, field, pressure, pitot, altitude, temperature.
            let acceleration = bounds.force.limit();
            let rate = bounds.rate_limit();
            let limits = [
                acceleration,
                acceleration,
                acceleration,
                rate,
                rate,
                rate,
                1e-4,
                1e-4,
                1e-4,
                0.01,
                0.001,
                0.002,
                1e-4,
            ];
            for (i, limit) in limits.into_iter().enumerate() {
                close(
                    at,
                    "HIL_SENSOR",
                    f32_at(r, 8 + 4 * i),
                    f32_at(c, 8 + 4 * i),
                    limit,
                );
            }
            assert_eq!(&r[70..74], &c[70..74], "{at}: sensor flags");
        }
        if message_id(r) == HIL_GPS.0 {
            assert_gps(at, r, c, bounds);
        }
    }
}

/// One landing site: where the vehicle starts and the ground height.
#[derive(Debug, Clone, Copy)]
struct Site {
    latitude_deg: f64,
    longitude_deg: f64,
    heading_deg: f64,
    ground_m: f64,
}

const N51_SITE: Site = Site {
    latitude_deg: N51.latitude_deg,
    longitude_deg: N51.longitude_deg,
    heading_deg: N51.heading_deg,
    ground_m: GROUND_M,
};

fn x500_paths(aircraft_dir: std::path::PathBuf) -> ModelPaths {
    ModelPaths {
        root: option_env!("PILOTAGE_JSBSIM_ROOT").map(std::path::PathBuf::from),
        aircraft_dir,
        model: "x500".to_owned(),
    }
}

/// The widest bounds of one landing.
#[derive(Debug, Clone, Copy, Default)]
struct Widest {
    force_m_s2: f64,
    rate_rad_s: f64,
}

/// Climbs, cuts the motors, falls onto the gear, and settles, with the
/// Rust engine on `rust_dir` and the C++ engine on the bundled model,
/// comparing truth and frames at every step. Returns the two engines
/// at rest and the widest bounds the comparison used.
fn land(site: Site, step_ms: u32, rust_dir: std::path::PathBuf) -> ([Simulation; 2], Widest) {
    let mut world = World::new(
        N51,
        UniformTerrain {
            elevation_m: site.ground_m,
        },
        MagneticEpoch::from_decimal_year(2026.8),
    );
    let start = Start {
        latitude_deg: site.latitude_deg,
        longitude_deg: site.longitude_deg,
        heading_deg: site.heading_deg,
    };
    let rust_struts = struts(&rust_dir, "x500");
    let mut config = vehicle(Aircraft::X500, x500_paths(rust_dir), start, None, step_ms);
    let mut rust = Simulation::new(&config, &mut world).unwrap();
    config.engine = EngineKind::Cpp;
    config.paths = x500_paths(Aircraft::X500.bundled_dir());
    let mut cpp = Simulation::new(&config, &mut world).unwrap();
    let cpp_struts = struts(&Aircraft::X500.bundled_dir(), "x500");
    let mut contact = Contact::new(&mut rust, rust_struts, &mut cpp, cpp_struts);
    let (mut r_encoder, mut c_encoder) = (Encoder::default(), Encoder::default());
    let mut peak = 0.0f64;
    let mut widest = Widest::default();
    for step in 0..8_000 / step_ms {
        let command = match step * step_ms {
            0..250 => controls(1.0, true),
            250..1_250 => controls(0.82, true),
            1_250..3_000 => controls(0.64, true),
            3_000..4_000 => controls(0.0, true),
            _ => controls(1.0, false),
        };
        let r = rust
            .step(Some(&command), &mut world, &mut r_encoder)
            .unwrap();
        let c = cpp
            .step(Some(&command), &mut world, &mut c_encoder)
            .unwrap();
        let (r_truth, c_truth) = (rust.truth(), cpp.truth());
        let step_s = f64::from(step_ms) / 1_000.0;
        let bounds = contact.bounds(&mut rust, &mut cpp, &r_truth, &c_truth, step_s);
        let at = format!(
            "{}°, {}°, {} m, {step_ms} ms, step {step}",
            site.latitude_deg, site.longitude_deg, site.ground_m
        );
        assert_same_state(&at, &r_truth, &c_truth, bounds);
        assert_frames(&at, &r, &c, bounds);
        assert!(
            bounds.compression.holds(),
            "{at}: contact compression: |Δx| {} m, limit {} m",
            bounds.compression.difference_m,
            bounds.compression.limit_m
        );
        assert!(
            bounds.rate_limit() <= RATE_CAP_RAD_S,
            "{at}: the comparison no longer constrains the body rates: {}, cap {RATE_CAP_RAD_S}",
            bounds.describe_rate()
        );
        assert!(
            bounds.force.limit() <= FORCE_CAP_M_S2,
            "{at}: the comparison no longer constrains the specific force: {}, cap {FORCE_CAP_M_S2}",
            bounds.force.describe()
        );
        widest.force_m_s2 = widest.force_m_s2.max(bounds.force.limit());
        widest.rate_rad_s = widest.rate_rad_s.max(bounds.rate_limit());
        peak = peak.max(r_truth.altitude_m - site.ground_m);
    }
    assert!(peak > 0.5, "the vehicle must leave the ground: {peak}");
    ([rust, cpp], widest)
}

#[test]
fn landing_truth_and_sensor_frames_agree_at_all_step_rates_and_sites() {
    // Include the southern hemisphere, high latitude, the date line,
    // negative terrain height, high terrain, and a heading near zero.
    let mut report = String::from("widest contact bounds (cap: force, rate):\n");
    for site in [
        N51_SITE,
        Site {
            latitude_deg: -33.9,
            longitude_deg: 179.9999,
            heading_deg: 359.99,
            ground_m: -200.0,
        },
        Site {
            latitude_deg: 78.0,
            longitude_deg: -179.9999,
            heading_deg: 0.0,
            ground_m: 3_000.0,
        },
    ] {
        for step_ms in [1, 2, 4] {
            let (mut simulations, widest) = land(site, step_ms, Aircraft::X500.bundled_dir());
            report.push_str(&format!(
                "  {}°, {step_ms} ms: force {:.4} m/s² ({FORCE_CAP_M_S2}), rate {:.2e} rad/s ({RATE_CAP_RAD_S})\n",
                site.latitude_deg, widest.force_m_s2, widest.rate_rad_s
            ));
            for simulation in &mut simulations {
                let truth = simulation.truth();
                assert!((truth.altitude_m - site.ground_m - 0.12).abs() < 0.02);
                assert!(truth.velocity_ned.iter().all(|v| v.abs() < 0.01));
            }
        }
    }
    // The widest bounds are a result of the test. `cargo test -- --nocapture`
    // shows them.
    std::io::Write::write_all(&mut std::io::stderr(), report.as_bytes()).unwrap();
}

/// A copy of the bundled X500 with the value of the first `count`
/// elements that open with `open` (for example `<spring_coeff ` or
/// `<x>`) inside the contacts scaled by `scale`, in a directory of its
/// own.
fn scaled_x500(open: &str, scale: f64, count: usize) -> std::path::PathBuf {
    let source = std::fs::read_to_string(Aircraft::X500.bundled_dir().join("x500.xml")).unwrap();
    let contacts = source.find("<ground_reactions>").unwrap();
    let mut scaled = source[..contacts].to_owned();
    let mut rest = &source[contacts..];
    for _ in 0..count {
        let Some(at) = rest.find(open) else {
            break;
        };
        let open_end = rest[at..].find('>').unwrap() + at + 1;
        let close = rest[open_end..].find('<').unwrap() + open_end;
        let value: f64 = rest[open_end..close].trim().parse().unwrap();
        scaled.push_str(&rest[..open_end]);
        scaled.push_str(&format!("{}", value * scale));
        rest = &rest[close..];
    }
    scaled.push_str(rest);
    assert_ne!(scaled, source, "the contacts have no {open}");
    let name: String = open.chars().filter(char::is_ascii_alphanumeric).collect();
    let dir = std::env::temp_dir().join(format!(
        "pilotage-x500-{name}-{scale}-{count}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x500.xml"), scaled).unwrap();
    dir
}

/// The failure message of a landing over a Rust-side model in `dir`.
fn landing_failure(dir: std::path::PathBuf) -> String {
    let run = std::panic::catch_unwind(|| {
        land(N51_SITE, 1, dir.clone());
    });
    std::fs::remove_dir_all(&dir).unwrap();
    let message = run.expect_err("the landing comparison must fail");
    message
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default()
}

/// A gear that is not the C++ engine's gear fails the landing comparison
/// on the first check that can see the change:
///
/// - Every strut 5 % more damped: the strut force differs at once while
///   the states still agree, so the specific force fails.
/// - Every strut 5 % stiffer: the vehicle sits higher at rest, where the
///   forces of both engines carry the same weight, so the contact
///   compression fails at the first sample.
#[test]
fn a_five_percent_change_of_every_strut_fails_the_landing_comparison() {
    let damped = landing_failure(scaled_x500("<damping_coeff ", 1.05, 4));
    assert!(damped.contains(": specific force: "), "{damped}");
    let stiff = landing_failure(scaled_x500("<spring_coeff ", 1.05, 4));
    assert!(stiff.contains(": contact compression: "), "{stiff}");
}

/// An asymmetric gear fails the landing comparison:
///
/// - One strut 5 % more damped: the torque differs while the attitude
///   still agrees, so the body rate fails.
/// - One strut 5 % stiffer, or one contact 5 % further out: the vehicle
///   sits tilted at rest, so the contact compression fails at the first
///   sample.
#[test]
fn a_five_percent_change_of_one_strut_fails_the_landing_comparison() {
    let damped = landing_failure(scaled_x500("<damping_coeff ", 1.05, 1));
    assert!(damped.contains(": rate: "), "{damped}");
    for open in ["<spring_coeff ", "<x>"] {
        let text = landing_failure(scaled_x500(open, 1.05, 1));
        assert!(text.contains(": contact compression: "), "{open}: {text}");
    }
}

/// A course at no ground speed has no direction, so any two courses
/// agree; at an ordinary speed the course bound stays near five
/// centidegrees.
#[test]
fn the_course_bound_is_open_at_rest_and_tight_in_motion() {
    let bounds = Bounds {
        horizontal_velocity_m_s: 1e-6,
        ..Bounds::AIRBORNE
    };
    assert!(bounds.course_limit_cdeg(0.0).is_infinite());
    let moving = bounds.course_limit_cdeg(5.0);
    assert!((5.0..5.01).contains(&moving), "{moving}");
}
