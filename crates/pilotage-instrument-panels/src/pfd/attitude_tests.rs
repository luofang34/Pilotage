#![allow(clippy::expect_used, clippy::panic)]
//! Unusual-attitude (ATT-01) and frame-adapter (FRAME-01) scene tests,
//! split from the base PFD suite to honor the file-size wall.

use pilotage_instrument_scene::{Cmd, SceneCmds};
use pilotage_instrument_state::{
    AirData, AircraftState, Attitude, FreshnessPolicy, Kinematics, PanelData, Quat, Stamped,
    resolve,
};

use super::tests::{render, texts};
use super::{PfdConfig, VSpeeds};

/// f32 ZYX euler → quaternion for orientation fixtures.
fn quat_euler(roll_deg: f32, pitch_deg: f32, yaw_deg: f32) -> Quat {
    let d = core::f32::consts::PI / 180.0;
    let (r, p, y) = (roll_deg * d / 2.0, pitch_deg * d / 2.0, yaw_deg * d / 2.0);
    let (cr, sr) = (libm::cosf(r), libm::sinf(r));
    let (cp, sp) = (libm::cosf(p), libm::sinf(p));
    let (cy, sy) = (libm::cosf(y), libm::sinf(y));
    Quat {
        w: cr * cp * cy + sr * sp * sy,
        x: sr * cp * cy - cr * sp * sy,
        y: cr * sp * cy + sr * cp * sy,
        z: cr * cp * sy - sr * sp * cy,
    }
}

fn oriented(roll_deg: f32, pitch_deg: f32) -> PanelData {
    let mut state = AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: quat_euler(roll_deg, pitch_deg, 0.0),
                rates_rps: [0.0, 0.0, 0.02],
            }),
            age_ms: Some(20.0),
        },
        ..AircraftState::default()
    };
    state.quality = pilotage_instrument_state::EstimateQuality::Good;
    state.valid = pilotage_instrument_state::ValidFlags {
        attitude: true,
        rates: true,
        position: true,
        velocity: true,
        ..Default::default()
    };
    state.kinematics = flying_state_kinematics();
    state.air = flying_state_air();
    resolve(&state, &FreshnessPolicy::default())
}

fn flying_state_kinematics() -> Stamped<Kinematics> {
    Stamped {
        data: Some(Kinematics {
            pos_ned_m: [0.0, 0.0, -300.0],
            vel_ned_mps: [20.0, 0.0, -1.0],
        }),
        age_ms: Some(20.0),
    }
}

fn flying_state_air() -> Stamped<AirData> {
    Stamped {
        data: Some(AirData {
            ias_mps: Some(40.0),
            baro_setting_hpa: Some(1013.0),
        }),
        age_ms: Some(20.0),
    }
}

fn banded_cfg() -> PfdConfig {
    PfdConfig {
        v_speeds: Some(VSpeeds {
            vs0_kt: 40.0,
            vs_kt: 48.0,
            vfe_kt: 85.0,
            vno_kt: 129.0,
            vne_kt: 163.0,
        }),
        ..PfdConfig::default()
    }
}

fn count_cmds(scene: &[u8], mut hit: impl FnMut(&Cmd<'_>) -> bool) -> usize {
    SceneCmds::new(scene)
        .expect("valid scene")
        .map(|c| c.expect("valid command"))
        .filter(|c| hit(c))
        .count()
}

fn chevrons_in(scene: &[u8]) -> usize {
    count_cmds(scene, |c| matches!(c, Cmd::Polyline { .. }))
}

fn turn_rate_cue_in(scene: &[u8]) -> bool {
    count_cmds(
        scene,
        |c| matches!(c, Cmd::Line { x1, y1, .. } if *x1 == 178.0 && *y1 == 334.0),
    ) > 0
}

fn bands_in(scene: &[u8]) -> bool {
    count_cmds(
        scene,
        |c| matches!(c, Cmd::FillColor { color } if *color == pilotage_instrument_scene::Rgba8::rgb(0, 160, 0)),
    ) > 0
}

#[test]
fn normal_envelope_has_no_unusual_artifacts() {
    let scene = render(&oriented(10.0, 5.0), &banded_cfg());
    assert_eq!(chevrons_in(&scene), 0);
    assert!(turn_rate_cue_in(&scene));
    assert!(bands_in(&scene));
}

#[test]
fn declutter_follows_the_one_priority_table() {
    let normal = render(&oriented(10.0, 5.0), &banded_cfg());
    let decluttered = render(&oriented(70.0, 5.0), &banded_cfg());

    // Removed: turn-rate cue, speed bands, minor ladder rows.
    assert!(!turn_rate_cue_in(&decluttered));
    assert!(!bands_in(&decluttered));
    let lines = |scene: &[u8]| count_cmds(scene, |c| matches!(c, Cmd::Line { .. }));
    assert!(
        lines(&decluttered) < lines(&normal),
        "minor ladder rows removed"
    );

    // Preserved: primary attitude, airspeed, altitude readouts.
    let labels = texts(&decluttered);
    assert!(labels.iter().any(|t| t == "078"), "IAS kept: {labels:?}");
    assert!(labels.iter().any(|t| t == "980"), "ALT kept: {labels:?}");
}

#[test]
fn declutter_never_removes_alerts_or_failures() {
    let mut data = oriented(70.0, 5.0);
    data.ias_kt.status = pilotage_instrument_state::SignalStatus::Failed;
    data.roll_rad.status = pilotage_instrument_state::SignalStatus::Degraded;
    data.pitch_rad.status = pilotage_instrument_state::SignalStatus::Degraded;
    let scene = render(&data, &banded_cfg());
    let labels = texts(&scene);
    assert!(labels.iter().any(|t| t == "IAS"), "IAS failure flag kept");
    assert!(labels.iter().any(|t| t == "ATT"), "ATT caution kept");
}

#[test]
fn chevrons_point_toward_the_horizon() {
    let nose_high = render(&oriented(0.0, 55.0), &PfdConfig::default());
    assert_eq!(chevrons_in(&nose_high), 2, "nose-high chevrons drawn");
    let nose_low = render(&oriented(0.0, -35.0), &PfdConfig::default());
    assert_eq!(chevrons_in(&nose_low), 2, "nose-low chevrons drawn");
    let normal = render(&oriented(0.0, 20.0), &PfdConfig::default());
    assert_eq!(chevrons_in(&normal), 0);

    // Sense: nose-high apexes sit below their bases (+y toward the
    // horizon), nose-low mirrors.
    let apex_sign = |scene: &[u8]| {
        let mut signs = std::vec::Vec::new();
        for c in SceneCmds::new(scene).expect("scene") {
            if let Cmd::Polyline { points } = c.expect("cmd") {
                let base = points.get(0).expect("base")[1];
                let apex = points.get(1).expect("apex")[1];
                signs.push(apex > base);
            }
        }
        signs
    };
    assert!(apex_sign(&nose_high).iter().all(|&down| down));
    assert!(apex_sign(&nose_low).iter().all(|&down| !down));
}

#[test]
fn every_extreme_orientation_emits_finite_layered_scenes() {
    for (roll, pitch) in [
        (0.0f32, 89.0f32),
        (0.0, 90.0),
        (0.0, 91.0),
        (10.0, 90.0),
        (180.0, 0.0),
        (179.0, -20.0),
        (90.0, 45.0),
        (-90.0, -45.0),
        (65.0, 30.0),
        (-66.0, -50.0),
    ] {
        let scene = render(&oriented(roll, pitch), &banded_cfg());
        let report =
            pilotage_instrument_scene::validate_layers(&scene).expect("layered at extremes");
        assert!(report.contains(pilotage_instrument_scene::LayerId::Attitude));
        for c in SceneCmds::new(&scene).expect("scene") {
            if let Cmd::Line { x1, y1, x2, y2 } = c.expect("cmd") {
                assert!(
                    x1.is_finite() && y1.is_finite() && x2.is_finite() && y2.is_finite(),
                    "non-finite line at roll {roll} pitch {pitch}"
                );
            }
        }
    }
}

// ---- FRAME-01 NED adapter compatibility ----------------------------------------

#[test]
fn ned_adapter_output_is_byte_identical_to_the_direct_path() {
    use pilotage_frames::{ClockDomain, Epoch, FrameId, Tagged, TimeScale, ned_attitude};

    // The same physical attitude, once fed directly and once through
    // the explicit NED reference selection: the PFD scene bytes must be
    // identical — the adapter changes where the frame choice is made,
    // never what the aircraft display shows.
    let quat = quat_euler(12.0, 6.0, 0.0);
    let tagged = Tagged {
        frame: FrameId::Ned,
        epoch: Epoch {
            clock: ClockDomain::Simulation,
            scale: TimeScale::Monotonic,
            nanos: 1,
        },
        meta: (),
        value: quat,
    };
    let adapted = ned_attitude(&tagged).expect("NED reference accepted");
    assert_eq!(adapted, quat);

    let direct = render(&oriented(12.0, 6.0), &banded_cfg());
    let mut via_adapter_state = AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: adapted,
                rates_rps: [0.0, 0.0, 0.02],
            }),
            age_ms: Some(20.0),
        },
        ..AircraftState::default()
    };
    via_adapter_state.quality = pilotage_instrument_state::EstimateQuality::Good;
    via_adapter_state.valid = pilotage_instrument_state::ValidFlags {
        attitude: true,
        rates: true,
        position: true,
        velocity: true,
        ..Default::default()
    };
    via_adapter_state.kinematics = flying_state_kinematics();
    via_adapter_state.air = flying_state_air();
    let via_adapter = render(
        &resolve(&via_adapter_state, &FreshnessPolicy::default()),
        &banded_cfg(),
    );
    assert_eq!(
        direct, via_adapter,
        "scene bytes identical through the adapter"
    );

    // A non-NED reference is refused, never silently rendered as a
    // horizon.
    let inertial = Tagged {
        frame: FrameId::Eci,
        ..tagged
    };
    assert!(ned_attitude(&inertial).is_err());
}
