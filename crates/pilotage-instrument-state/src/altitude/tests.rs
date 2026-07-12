#![allow(clippy::expect_used, clippy::panic)]

use super::{AltitudeClass, AltitudeReference, GeoidModelId, OriginId};

#[test]
fn wire_classes_round_trip_and_unknown_fails_closed() {
    for class in [
        AltitudeClass::LocalRelative,
        AltitudeClass::BaroIndicated,
        AltitudeClass::Pressure,
        AltitudeClass::GeometricMsl,
        AltitudeClass::Agl,
    ] {
        assert_eq!(AltitudeClass::from_u8(class.to_u8()), class);
    }
    for unknown in [5u8, 6, 100, 254, 255] {
        assert_eq!(AltitudeClass::from_u8(unknown), AltitudeClass::Unknown);
    }
    assert_eq!(
        AltitudeClass::from_u8(AltitudeClass::Unknown.to_u8()),
        AltitudeClass::Unknown
    );
}

#[test]
fn every_reference_reports_its_class_and_label() {
    let cases: [(AltitudeReference, AltitudeClass, &str); 5] = [
        (
            AltitudeReference::LocalRelative {
                origin: OriginId(7),
            },
            AltitudeClass::LocalRelative,
            "REL",
        ),
        (
            AltitudeReference::BaroIndicated {
                applied_hpa: 1013.2,
            },
            AltitudeClass::BaroIndicated,
            "BARO",
        ),
        (AltitudeReference::Pressure, AltitudeClass::Pressure, "STD"),
        (
            AltitudeReference::GeometricMsl {
                model: GeoidModelId(1),
            },
            AltitudeClass::GeometricMsl,
            "MSL",
        ),
        (AltitudeReference::Agl, AltitudeClass::Agl, "AGL"),
    ];
    for (reference, class, label) in cases {
        assert_eq!(reference.class(), class);
        assert_eq!(class.label(), label);
    }
    assert_eq!(AltitudeClass::Unknown.label(), "REF");
}

#[test]
fn numerically_equal_values_in_different_references_are_incompatible() {
    // 1000 ft REL and 1000 ft BARO are different quantities; the only
    // sanctioned comparison is class equality, never value equality.
    let rel = AltitudeReference::LocalRelative {
        origin: OriginId(0),
    };
    let baro = AltitudeReference::BaroIndicated {
        applied_hpa: 1013.2,
    };
    assert_ne!(rel.class(), baro.class());
}
