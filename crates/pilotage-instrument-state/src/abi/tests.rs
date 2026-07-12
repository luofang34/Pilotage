#![allow(clippy::expect_used, clippy::panic)]

use super::{AbiError, STATE_ABI_SIZE, decode_state, encode_state};
use crate::aircraft::{
    AirData, AircraftState, Attitude, EstimateQuality, Kinematics, NavData, NavFromTo, NavSource,
    Selections, SnapshotCoherence, SnapshotMeta, Stamped, Wind,
};
use pilotage_frames::Quat;

fn full_state() -> AircraftState {
    AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat {
                    w: 0.9,
                    x: 0.1,
                    y: 0.2,
                    z: 0.3,
                },
                rates_rps: [0.01, 0.02, 0.03],
            }),
            age_ms: Some(42.0),
        },
        kinematics: Stamped {
            data: Some(Kinematics {
                pos_ned_m: [1.0, 2.0, -3.0],
                vel_ned_mps: [4.0, 5.0, -0.5],
            }),
            age_ms: Some(100.0),
        },
        air: Stamped {
            data: Some(AirData {
                ias_mps: None,
                baro_setting_hpa: Some(1013.25),
            }),
            age_ms: Some(7.0),
        },
        nav: Stamped {
            data: Some(NavData {
                source: NavSource::Gps,
                course_rad: 0.35,
                cdi_dots: -1.2,
                fromto: NavFromTo::To,
                vdev_dots: Some(0.4),
                dist_nm: Some(40.3),
            }),
            age_ms: Some(9.0),
        },
        wind: Stamped {
            data: Some(Wind {
                from_rad: 4.7,
                speed_mps: 7.2,
            }),
            age_ms: Some(11.0),
        },
        selections: Selections {
            heading_bug_rad: 0.16,
            altitude_sel_m: Some(3048.0),
        },
        quality: EstimateQuality::Degraded,
        valid: crate::aircraft::ValidFlags {
            attitude: true,
            rates: false,
            position: true,
            velocity: true,
        },
        snapshot: SnapshotMeta {
            generation: u32::MAX,
            coherence: SnapshotCoherence::Coherent,
        },
    }
}

#[test]
fn round_trip_preserves_everything() {
    let state = full_state();
    let mut buf = [0u8; STATE_ABI_SIZE];
    encode_state(&state, &mut buf).expect("fits");
    let back = decode_state(&buf).expect("decodes");
    assert_eq!(back, state);
}

#[test]
fn empty_state_round_trips_as_all_absent() {
    let state = AircraftState::default();
    let mut buf = [0u8; STATE_ABI_SIZE];
    encode_state(&state, &mut buf).expect("fits");
    let back = decode_state(&buf).expect("decodes");
    assert!(back.attitude.data.is_none());
    assert!(back.air.data.is_none());
    assert!(back.nav.data.is_none());
    assert!(back.wind.data.is_none());
}

#[test]
fn short_buffer_is_truncated() {
    let mut buf = [0u8; STATE_ABI_SIZE];
    encode_state(&full_state(), &mut buf).expect("fits");
    assert_eq!(
        decode_state(&buf[..STATE_ABI_SIZE - 1]).err(),
        Some(AbiError::Truncated)
    );
}

#[test]
fn wrong_version_is_rejected() {
    let mut buf = [0u8; STATE_ABI_SIZE];
    encode_state(&full_state(), &mut buf).expect("fits");
    buf[0] = 99;
    assert_eq!(
        decode_state(&buf).err(),
        Some(AbiError::BadVersion { found: 99 })
    );
}

// ---- VAL-01 fail-safe wire decoding --------------------------------------------

#[test]
fn unknown_wire_values_decode_fail_safe_not_benign() {
    let mut block = [0u8; STATE_ABI_SIZE];
    encode_state(&full_state(), &mut block).expect("encodes");
    // Poke values outside every known enum range.
    block[84] = 7; // quality
    block[86] = 9; // nav source
    block[87] = 9; // nav from/to
    block[124] = 9; // coherence
    let state = decode_state(&block).expect("decodes");
    assert_eq!(state.quality, EstimateQuality::Unknown);
    assert_eq!(
        state.nav.data.expect("nav present").source,
        NavSource::Unknown
    );
    assert_eq!(
        state.nav.data.expect("nav present").fromto,
        NavFromTo::Unknown
    );
    assert_eq!(state.snapshot.coherence, SnapshotCoherence::Unknown);
}

#[test]
fn unknown_variants_round_trip_as_explicit_unknown() {
    let mut state = full_state();
    state.quality = EstimateQuality::Unknown;
    if let Some(nav) = state.nav.data.as_mut() {
        nav.source = NavSource::Unknown;
        nav.fromto = NavFromTo::Unknown;
    }
    state.snapshot.coherence = SnapshotCoherence::Unknown;
    let mut block = [0u8; STATE_ABI_SIZE];
    encode_state(&state, &mut block).expect("encodes");
    assert_eq!(block[84], 255);
    assert_eq!(block[86], 255);
    assert_eq!(block[87], 255);
    assert_eq!(block[124], 255);
    let decoded = decode_state(&block).expect("decodes");
    assert_eq!(decoded.quality, EstimateQuality::Unknown);
    assert_eq!(decoded.snapshot.coherence, SnapshotCoherence::Unknown);
}

#[test]
fn zeroed_metadata_never_decodes_as_trusted() {
    // A block whose metadata bytes are all zero: quality 0 is an
    // explicit Good declaration (the wire's zero value), but the valid
    // flags are all unset — nothing is declared valid.
    let mut block = [0u8; STATE_ABI_SIZE];
    encode_state(&AircraftState::default(), &mut block).expect("encodes");
    assert_eq!(block[85], 0, "default flags declare nothing valid");
    assert_eq!(block[84], 255, "default quality encodes unknown");
    let state = decode_state(&block).expect("decodes");
    assert!(!state.valid.attitude);
    assert_eq!(state.quality, EstimateQuality::Unknown);
}
