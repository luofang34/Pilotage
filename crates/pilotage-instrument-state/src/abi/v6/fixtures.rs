//! Shared ABI posture fixtures: the states behind the committed golden
//! frames in `clients/web/fixtures/`.
//!
//! Three deliberately different group sets (ADR-0026): a fully fed
//! source, a data-gateway bridge, and a flight controller. Neither
//! posture is a subset of the other, which is exactly what the tagged
//! wire must express without dead fields. Values are fixed, asymmetric,
//! and time-free so `cargo xtask gen-state-fixture` reproduces identical
//! bytes on every run; the Rust golden test and the JS writer test pin
//! against the same committed hex.

use crate::aircraft::{
    AirData, AircraftState, Attitude, EstimateQuality, Kinematics, NavData, NavFromTo, NavSource,
    Selections, SnapshotCoherence, SnapshotMeta, Stamped, ValidFlags, Wind,
};
use crate::altitude::{AltitudeClass, AltitudeDeclaration, GeoidModelId, OriginId};
use crate::dynamics::{DynSample, TurnBasis, TurnSample};
use crate::heading::{HeadingReference, HeadingSample, MagneticVariation, VariationSourceId};
use crate::ident::IdentStr;
use crate::monitor_text::{MonitorText, TextLine};
use pilotage_frames::Quat;

fn ident(text: &str) -> IdentStr {
    IdentStr::new(text).unwrap_or(IdentStr::EMPTY)
}

fn stamped<T>(data: T, age_ms: f32) -> Stamped<T> {
    Stamped {
        data: Some(data),
        age_ms: Some(age_ms),
    }
}

fn attitude(quat: [f32; 4], rates_rps: [f32; 3], age_ms: f32) -> Stamped<Attitude> {
    stamped(
        Attitude {
            quat: Quat {
                w: quat[0],
                x: quat[1],
                y: quat[2],
                z: quat[3],
            },
            rates_rps,
        },
        age_ms,
    )
}

fn kinematics(pos_ned_m: [f32; 3], vel_ned_mps: [f32; 3], age_ms: f32) -> Stamped<Kinematics> {
    stamped(
        Kinematics {
            pos_ned_m,
            vel_ned_mps,
        },
        age_ms,
    )
}

fn nav(course_rad: f32, cdi_dots: f32, to: &str, from: &str, age_ms: f32) -> Stamped<NavData> {
    stamped(
        NavData {
            source: NavSource::Gps,
            course_rad,
            cdi_dots,
            fromto: NavFromTo::To,
            vdev_dots: None,
            dist_nm: None,
            course_reference: HeadingReference::SimLocalTrue,
            to_ident: ident(to),
            from_ident: ident(from),
        },
        age_ms,
    )
}

fn heading(heading_rad: f32, reference: HeadingReference, age_ms: f32) -> Stamped<HeadingSample> {
    stamped(
        HeadingSample {
            heading_rad,
            reference,
        },
        age_ms,
    )
}

fn variation(east_positive_rad: f32, source: u8, age_ms: f32) -> Stamped<MagneticVariation> {
    stamped(
        MagneticVariation {
            east_positive_rad,
            source: VariationSourceId(source),
        },
        age_ms,
    )
}

fn dynamics(rate_rps: f32, basis: TurnBasis, lateral_mps2: f32, age_ms: f32) -> Stamped<DynSample> {
    stamped(
        DynSample {
            turn: Some(TurnSample { rate_rps, basis }),
            lateral_mps2: Some(lateral_mps2),
        },
        age_ms,
    )
}

fn air(ias_mps: f32, baro_setting_hpa: f32, age_ms: f32) -> Stamped<AirData> {
    stamped(
        AirData {
            ias_mps: Some(ias_mps),
            baro_setting_hpa: Some(baro_setting_hpa),
        },
        age_ms,
    )
}

fn all_valid() -> ValidFlags {
    ValidFlags {
        attitude: true,
        rates: true,
        position: true,
        velocity: true,
        heading: true,
        variation: true,
        turn: true,
        slip: true,
    }
}

fn monitor(revision: u32, texts: &[&str]) -> MonitorText {
    let mut lines = [TextLine::EMPTY; MonitorText::MAX_LINES];
    for (slot, text) in lines.iter_mut().zip(texts) {
        *slot = TextLine::new(text).unwrap_or(TextLine::EMPTY);
    }
    MonitorText::new(revision, &lines[..texts.len().min(MonitorText::MAX_LINES)])
        .unwrap_or_default()
}

fn baro_altitude(sample_m: f32, origin: u32) -> AltitudeDeclaration {
    AltitudeDeclaration {
        reference_class: AltitudeClass::BaroIndicated,
        sample_m: Some(sample_m),
        geoid_model: GeoidModelId::UNDECLARED,
        origin: OriginId(origin),
    }
}

/// Every group present, asymmetric on purpose.
pub fn full() -> AircraftState {
    let mut nav = nav(0.6, 0.7, "WPT-2", "KMRY", 80.0);
    if let Some(data) = nav.data.as_mut() {
        data.vdev_dots = Some(-0.4);
        data.dist_nm = Some(12.4);
    }
    AircraftState {
        attitude: attitude([0.5, 0.5, 0.5, 0.5], [0.02, -0.01, 0.05], 80.0),
        kinematics: kinematics([1200.0, 340.0, -305.0], [52.0, 9.0, -2.0], 80.0),
        air: air(53.0, 1013.2, 80.0),
        nav,
        wind: stamped(
            Wind {
                from_rad: 2.1,
                speed_mps: 7.5,
            },
            80.0,
        ),
        selections: Selections {
            heading_bug_rad: 1.0,
            heading_bug_reference: HeadingReference::SimLocalTrue,
            altitude_sel_m: Some(500.0),
            altitude_sel_class: AltitudeClass::LocalRelative,
            altitude_sel_origin: OriginId(7),
            altitude_sel_model: GeoidModelId::UNDECLARED,
            baro_sel_hpa: Some(1013.2),
        },
        quality: EstimateQuality::Good,
        valid: all_valid(),
        snapshot: SnapshotMeta {
            generation: 42,
            coherence: SnapshotCoherence::Coherent,
        },
        altitude: baro_altitude(950.0, 7),
        heading: heading(0.35, HeadingReference::SimLocalTrue, 90.0),
        variation: variation(0.15, 3, 120.0),
        dynamics: dynamics(0.05, TurnBasis::HeadingRate, 0.3, 85.0),
        monitor_text: stamped(monitor(9, &["ENG 1 OK", "FUEL 82.5"]), 500.0),
    }
}

/// A certified-panel bridge (ADR-0026 data-gateway): GNSS kinematics,
/// pressure altitude, and flight-plan guidance — no airspeed, no
/// attitude, no magnetic heading, no dynamics. Absent groups are absent
/// tags, not zeroed fields.
pub fn data_gateway() -> AircraftState {
    let mut nav = nav(1.2, -0.3, "WPT-3", "GATE-A", 150.0);
    if let Some(data) = nav.data.as_mut() {
        data.dist_nm = Some(8.7);
        data.course_reference = HeadingReference::True;
    }
    AircraftState {
        kinematics: kinematics([-2500.0, 800.0, -1200.0], [61.0, -4.0, 1.5], 120.0),
        nav,
        quality: EstimateQuality::Good,
        valid: ValidFlags {
            position: true,
            velocity: true,
            ..ValidFlags::default()
        },
        snapshot: SnapshotMeta {
            generation: 7,
            coherence: SnapshotCoherence::Insufficient,
        },
        altitude: AltitudeDeclaration {
            reference_class: AltitudeClass::Pressure,
            sample_m: Some(1150.0),
            geoid_model: GeoidModelId::UNDECLARED,
            origin: OriginId(0),
        },
        ..AircraftState::default()
    }
}

/// A flight controller: attitude, rates, air data, wind, heading,
/// variation, and dynamics — no flight-plan guidance.
pub fn flight_controller() -> AircraftState {
    AircraftState {
        attitude: attitude([1.0, 0.0, 0.0, 0.0], [0.01, 0.0, -0.02], 40.0),
        kinematics: kinematics([10.0, -20.0, -80.0], [21.0, 3.0, -0.5], 40.0),
        air: air(39.0, 1020.5, 45.0),
        wind: stamped(
            Wind {
                from_rad: 0.8,
                speed_mps: 4.2,
            },
            200.0,
        ),
        selections: Selections {
            heading_bug_rad: 2.4,
            heading_bug_reference: HeadingReference::Magnetic,
            altitude_sel_m: None,
            altitude_sel_class: AltitudeClass::LocalRelative,
            altitude_sel_origin: OriginId(0),
            altitude_sel_model: GeoidModelId::UNDECLARED,
            baro_sel_hpa: Some(1020.5),
        },
        quality: EstimateQuality::Good,
        valid: all_valid(),
        snapshot: SnapshotMeta {
            generation: 991,
            coherence: SnapshotCoherence::Coherent,
        },
        altitude: baro_altitude(320.0, 0),
        heading: heading(1.9, HeadingReference::Magnetic, 60.0),
        variation: variation(-0.05, 2, 60.0),
        dynamics: dynamics(-0.02, TurnBasis::TrackRate, -0.1, 50.0),
        ..AircraftState::default()
    }
}
