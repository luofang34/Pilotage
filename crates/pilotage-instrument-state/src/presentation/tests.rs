#![allow(clippy::expect_used, clippy::panic)]

use core::f32::consts::PI;

use super::{
    AirframeDisplayProfile, ChevronSense, Hysteresis, ProfileError, ProfileLimits,
    UnusualAttitudeState, down_in_body,
};
use pilotage_frames::Quat;

const DEG: f32 = PI / 180.0;

/// f64 reference quaternion from ZYX Euler angles, normalized.
fn quat_f64(roll_deg: f64, pitch_deg: f64, yaw_deg: f64) -> Quat {
    let (r, p, y) = (
        roll_deg.to_radians() / 2.0,
        pitch_deg.to_radians() / 2.0,
        yaw_deg.to_radians() / 2.0,
    );
    let (cr, sr) = (r.cos(), r.sin());
    let (cp, sp) = (p.cos(), p.sin());
    let (cy, sy) = (y.cos(), y.sin());
    let w = cr * cp * cy + sr * sp * sy;
    let x = sr * cp * cy - cr * sp * sy;
    let yq = cr * sp * cy + sr * cp * sy;
    let z = cr * cp * sy - sr * sp * cy;
    let n = (w * w + x * x + yq * yq + z * z).sqrt();
    Quat {
        w: (w / n) as f32,
        x: (x / n) as f32,
        y: (yq / n) as f32,
        z: (z / n) as f32,
    }
}

/// Independent f64 down-vector projection for the same quaternion.
fn down_f64(q: Quat) -> (f64, f64, f64) {
    let (w, x, y, z) = (
        f64::from(q.w),
        f64::from(q.x),
        f64::from(q.y),
        f64::from(q.z),
    );
    (
        2.0 * (x * z - w * y),
        2.0 * (y * z + w * x),
        1.0 - 2.0 * (x * x + y * y),
    )
}

fn neg(q: Quat) -> Quat {
    Quat {
        w: -q.w,
        x: -q.x,
        y: -q.y,
        z: -q.z,
    }
}

fn step_fresh(q: Quat) -> super::AttitudePresentation {
    let mut state = UnusualAttitudeState::default();
    state.step(q, &AirframeDisplayProfile::simulator())
}

/// The sky direction the rendered (pitch, bank) pair implies, as a unit
/// down-vector reconstruction; comparing it against the true down vector
/// proves the rendered geometry tracks physical orientation.
fn down_from_display(pitch: f32, bank: f32) -> (f64, f64, f64) {
    let (p, b) = (f64::from(pitch), f64::from(bank));
    (-p.sin(), b.sin() * p.cos(), b.cos() * p.cos())
}

#[test]
fn all_24_cube_rotations_match_the_f64_reference() {
    // Every proper rotation with axis-aligned columns: signed
    // permutation matrices with determinant +1.
    let perms = [
        [0, 1, 2],
        [1, 2, 0],
        [2, 0, 1],
        [0, 2, 1],
        [2, 1, 0],
        [1, 0, 2],
    ];
    let mut checked = 0;
    for (pi, perm) in perms.iter().enumerate() {
        let perm_sign: f64 = if pi < 3 { 1.0 } else { -1.0 };
        for signs in 0..8u8 {
            let s = [
                if signs & 1 != 0 { -1.0 } else { 1.0 },
                if signs & 2 != 0 { -1.0 } else { 1.0 },
                if signs & 4 != 0 { -1.0 } else { 1.0 },
            ];
            let det: f64 = perm_sign * s[0] * s[1] * s[2];
            if det < 0.0 {
                continue;
            }
            // Rotation matrix R with R[i][perm[i]] = s[i]; quaternion via
            // the robust f64 Shepperd branch.
            let mut m = [[0.0f64; 3]; 3];
            for i in 0..3 {
                m[i][perm[i]] = s[i];
            }
            let q = quat_from_matrix_f64(&m);
            let (dx, dy, dz) = down_in_body(q);
            // The true down vector in body coordinates is the matrix's
            // third row.
            for (got, want) in [(dx, m[2][0]), (dy, m[2][1]), (dz, m[2][2])] {
                assert!(
                    (f64::from(got) - want).abs() < 1e-6,
                    "rotation {perm:?}/{signs:#05b}: {got} vs {want}"
                );
            }
            let p = step_fresh(q);
            assert!(p.bank_rad.is_finite() && p.pitch_rad.is_finite());
            checked += 1;
        }
    }
    assert_eq!(checked, 24, "the full proper cube-rotation group");
}

/// Robust f64 rotation-matrix→quaternion (largest-component branch).
fn quat_from_matrix_f64(m: &[[f64; 3]; 3]) -> Quat {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let (w, x, y, z) = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        (
            s / 4.0,
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
        )
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        (
            (m[2][1] - m[1][2]) / s,
            s / 4.0,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
        )
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        (
            (m[0][2] - m[2][0]) / s,
            (m[0][1] + m[1][0]) / s,
            s / 4.0,
            (m[1][2] + m[2][1]) / s,
        )
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        (
            (m[1][0] - m[0][1]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            s / 4.0,
        )
    };
    Quat {
        w: w as f32,
        x: x as f32,
        y: y as f32,
        z: z as f32,
    }
}

#[test]
fn q_and_negated_q_are_bit_identical() {
    for &(roll, pitch, yaw) in DENSE_GRID {
        let q = quat_f64(roll, pitch, yaw);
        let a = step_fresh(q);
        let b = step_fresh(neg(q));
        assert_eq!(a, b, "roll {roll} pitch {pitch} yaw {yaw}");
        assert_eq!(a.bank_rad.to_bits(), b.bank_rad.to_bits());
        assert_eq!(a.pitch_rad.to_bits(), b.pitch_rad.to_bits());
    }
}

/// The issue's dense bounded grid: every listed pitch and bank magnitude,
/// both signs, with off-axis yaw so no case degenerates by construction.
const DENSE_GRID: &[(f64, f64, f64)] = &{
    const PITCHES: [f64; 9] = [0.0, 20.0, 30.0, 50.0, 89.0, 90.0, 91.0, 179.0, 180.0];
    const BANKS: [f64; 8] = [0.0, 64.0, 65.0, 66.0, 89.0, 90.0, 179.0, 180.0];
    let mut grid = [(0.0, 0.0, 0.0); PITCHES.len() * BANKS.len() * 4];
    let mut i = 0;
    let mut pi = 0;
    while pi < PITCHES.len() {
        let mut bi = 0;
        while bi < BANKS.len() {
            grid[i] = (BANKS[bi], PITCHES[pi], 33.0);
            grid[i + 1] = (-BANKS[bi], PITCHES[pi], 213.0);
            grid[i + 2] = (BANKS[bi], -PITCHES[pi], 33.0);
            grid[i + 3] = (-BANKS[bi], -PITCHES[pi], 213.0);
            i += 4;
            bi += 1;
        }
        pi += 1;
    }
    grid
};

#[test]
fn dense_so3_grid_is_finite_bounded_and_tracks_the_f64_reference() {
    for &(roll, pitch, yaw) in DENSE_GRID {
        let q = quat_f64(roll, pitch, yaw);
        let p = step_fresh(q);
        assert!(p.bank_rad.is_finite() && p.pitch_rad.is_finite());
        assert!(p.pitch_rad.abs() <= PI / 2.0 + 1e-6, "pitch bounded");
        assert!(p.bank_rad.abs() <= PI + 1e-6, "bank bounded");

        // Independent f64 projection reference: the rendered pair must
        // reconstruct the physical down vector (outside the held zone).
        let (rx, ry, rz) = down_f64(q);
        if f64::from(p.pitch_rad).cos() > 0.05 {
            let (gx, gy, gz) = down_from_display(p.pitch_rad, p.bank_rad);
            let dot = gx * rx + gy * ry + gz * rz;
            assert!(
                dot > 0.9999,
                "display geometry diverges at roll {roll} pitch {pitch}: dot {dot}"
            );
        }
    }
}

#[test]
fn continuous_pole_passage_keeps_the_sky_vector_continuous() {
    // Fly straight through the vertical: pitch 80°..100° in 0.25° steps
    // at fixed bank/yaw. The (pitch, bank) chart is singular at 90°, but
    // the reconstructed sky vector must track the true one closely on
    // both sides and never invert its meaning.
    let mut state = UnusualAttitudeState::default();
    let profile = AirframeDisplayProfile::simulator();
    // Inside the hold window the rendered bank is frozen, so the
    // rendered sky vector may deviate from truth by up to the window
    // width and snap back by about twice that at release. The bound is
    // that geometric limit (8° covers the 3° window, the release snap,
    // and one 0.25° step) — far from any sky/ground flip (180°).
    let bound = (8.0f64).to_radians().cos();
    let mut prev: Option<(f64, f64, f64)> = None;
    let mut step_deg = 80.0;
    while step_deg <= 100.0 {
        let q = quat_f64(10.0, step_deg, 40.0);
        let p = state.step(q, &profile);
        assert!(p.bank_rad.is_finite() && p.pitch_rad.is_finite());
        let (gx, gy, gz) = down_from_display(p.pitch_rad, p.bank_rad);
        let (tx, ty, tz) = down_f64(q);
        let truth = gx * tx + gy * ty + gz * tz;
        assert!(
            truth > bound,
            "rendered geometry left the hold-window bound at pitch {step_deg}: dot {truth}"
        );
        if let Some((px, py, pz)) = prev {
            let dot = gx * px + gy * py + gz * pz;
            assert!(
                dot > bound,
                "sky vector jumped at pitch {step_deg}: dot {dot}"
            );
        }
        prev = Some((gx, gy, gz));
        step_deg += 0.25;
    }
    // Past the pole the display must read inverted (bank beyond 90°),
    // not a relabeled sky.
    let past = state.step(quat_f64(10.0, 100.0, 40.0), &profile);
    assert!(past.inverted, "beyond the vertical reads inverted");
}

#[test]
fn inverted_flight_is_unambiguous() {
    let level_inverted = step_fresh(quat_f64(180.0, 0.0, 0.0));
    assert!(level_inverted.inverted);
    assert!((level_inverted.bank_rad.abs() - PI).abs() < 1e-3);
    assert!(level_inverted.pitch_rad.abs() < 1e-3);
    let upright = step_fresh(quat_f64(0.0, 0.0, 137.0));
    assert!(!upright.inverted);
}

#[test]
fn threshold_jitter_cannot_chatter() {
    // Oscillate ±0.5° around the 65° bank entry: one engagement, no
    // release until the 60° exit is crossed.
    let profile = AirframeDisplayProfile::simulator();
    let mut state = UnusualAttitudeState::default();
    let mut transitions = 0;
    let mut last = false;
    for i in 0..100 {
        let bank = if i % 2 == 0 { 65.4 } else { 64.6 };
        let p = state.step(quat_f64(bank, 5.0, 0.0), &profile);
        if p.high_bank != last {
            transitions += 1;
            last = p.high_bank;
        }
    }
    assert_eq!(transitions, 1, "one entry, no chatter");
    assert!(last, "still latched inside the hysteresis band");
    let released = state.step(quat_f64(59.0, 5.0, 0.0), &profile);
    assert!(!released.high_bank, "released below the exit threshold");
}

#[test]
fn tier_thresholds_follow_the_simulator_profile() {
    let at = |roll: f64, pitch: f64| step_fresh(quat_f64(roll, pitch, 0.0));
    assert!(!at(0.0, 29.0).unusual);
    assert!(at(0.0, 31.0).unusual && at(0.0, 31.0).nose_high);
    assert!(!at(0.0, -19.0).unusual);
    assert!(at(0.0, -21.0).unusual && at(0.0, -21.0).nose_low);
    assert!(!at(64.0, 0.0).unusual);
    assert!(at(66.0, 0.0).unusual && at(66.0, 0.0).high_bank);
    assert_eq!(at(0.0, 51.0).chevrons, Some(ChevronSense::HorizonBelow));
    assert_eq!(at(0.0, -31.0).chevrons, Some(ChevronSense::HorizonAbove));
    assert_eq!(at(0.0, 45.0).chevrons, None);
}

#[test]
fn bank_holds_through_the_vertical_and_resumes() {
    let profile = AirframeDisplayProfile::simulator();
    let mut state = UnusualAttitudeState::default();
    let before = state.step(quat_f64(15.0, 85.0, 0.0), &profile);
    let held = state.step(quat_f64(15.0, 89.5, 0.0), &profile);
    assert!(
        (held.bank_rad - before.bank_rad).abs() < 2.0 * DEG,
        "bank held near the pole"
    );
    let resumed = state.step(quat_f64(15.0, 30.0, 0.0), &profile);
    assert!((resumed.bank_rad - 15.0 * DEG).abs() < 0.5 * DEG);
}

#[test]
fn invalid_profiles_are_rejected() {
    let good = Hysteresis {
        entry: 30.0 * DEG,
        exit: 25.0 * DEG,
    };
    let base = ProfileLimits {
        unusual_pitch_up: good,
        unusual_pitch_down: good,
        unusual_bank: good,
        chevron_pitch_up: good,
        chevron_pitch_down: good,
        bank_hold_pitch: good,
    };
    assert!(AirframeDisplayProfile::new(base).is_ok());
    let mut inverted = base;
    inverted.unusual_bank = Hysteresis {
        entry: 25.0 * DEG,
        exit: 30.0 * DEG,
    };
    assert_eq!(
        AirframeDisplayProfile::new(inverted),
        Err(ProfileError::NoHysteresis)
    );
    let mut nan = base;
    nan.chevron_pitch_up = Hysteresis {
        entry: f32::NAN,
        exit: 25.0 * DEG,
    };
    assert_eq!(
        AirframeDisplayProfile::new(nan),
        Err(ProfileError::NonFinite)
    );
}
