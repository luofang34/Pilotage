#![allow(clippy::expect_used, clippy::panic)]

use core::f32::consts::PI;

use super::{
    ConversionFault, HeadingReference, MagneticVariation, VariationSourceId, convert_heading,
    shortest_angle_rad, wrap_2pi,
};

const DEG: f32 = PI / 180.0;

fn var(east_deg: f32) -> MagneticVariation {
    MagneticVariation {
        east_positive_rad: east_deg * DEG,
        source: VariationSourceId(1),
    }
}

#[test]
fn wrap_covers_the_359_to_1_degree_boundary_both_ways() {
    let delta = shortest_angle_rad(359.0 * DEG, 1.0 * DEG);
    assert!((delta - 2.0 * DEG).abs() < 1e-6, "359→1 is +2°, not −358°");
    let delta = shortest_angle_rad(1.0 * DEG, 359.0 * DEG);
    assert!((delta + 2.0 * DEG).abs() < 1e-6, "1→359 is −2°");
    assert!((wrap_2pi(-DEG) - 359.0 * DEG).abs() < 1e-5);
}

#[test]
fn variation_converts_across_north_in_both_signs() {
    // Magnetic 359° with 2°E variation is true 1°.
    let true_rad = convert_heading(
        359.0 * DEG,
        HeadingReference::Magnetic,
        HeadingReference::True,
        Some(&var(2.0)),
    )
    .expect("converts");
    assert!((true_rad - 1.0 * DEG).abs() < 1e-5);

    // Magnetic 1° with 2°W variation is true 359°.
    let true_rad = convert_heading(
        1.0 * DEG,
        HeadingReference::Magnetic,
        HeadingReference::True,
        Some(&var(-2.0)),
    )
    .expect("converts");
    assert!((true_rad - 359.0 * DEG).abs() < 1e-5);

    // The inverse subtracts: true 1° with 2°E variation is magnetic 359°.
    let mag_rad = convert_heading(
        1.0 * DEG,
        HeadingReference::True,
        HeadingReference::Magnetic,
        Some(&var(2.0)),
    )
    .expect("converts");
    assert!((mag_rad - 359.0 * DEG).abs() < 1e-5);
}

#[test]
fn true_north_pair_needs_no_variation_and_magnetic_crossing_requires_it() {
    let same = convert_heading(
        0.5,
        HeadingReference::SimLocalTrue,
        HeadingReference::True,
        None,
    );
    assert!((same.expect("true-north pair") - 0.5).abs() < 1e-6);

    let refused = convert_heading(
        0.5,
        HeadingReference::Magnetic,
        HeadingReference::True,
        None,
    );
    assert_eq!(refused, Err(ConversionFault::VariationUnavailable));
}

#[test]
fn stale_or_unattributed_variation_never_rotates_the_compass() {
    let undeclared = MagneticVariation {
        east_positive_rad: 0.1,
        source: VariationSourceId::UNDECLARED,
    };
    assert_eq!(
        convert_heading(
            0.5,
            HeadingReference::Magnetic,
            HeadingReference::True,
            Some(&undeclared),
        ),
        Err(ConversionFault::VariationUnavailable)
    );
    let non_finite = MagneticVariation {
        east_positive_rad: f32::NAN,
        source: VariationSourceId(1),
    };
    assert_eq!(
        convert_heading(
            0.5,
            HeadingReference::Magnetic,
            HeadingReference::True,
            Some(&non_finite),
        ),
        Err(ConversionFault::VariationUnavailable)
    );
}

#[test]
fn unknown_reference_refuses_conversion_and_round_trips_on_the_wire() {
    assert_eq!(
        convert_heading(0.5, HeadingReference::Unknown, HeadingReference::True, None),
        Err(ConversionFault::UnknownReference)
    );
    for reference in [
        HeadingReference::Magnetic,
        HeadingReference::True,
        HeadingReference::SimLocalTrue,
    ] {
        assert_eq!(HeadingReference::from_u8(reference.to_u8()), reference);
    }
    for unknown in [3u8, 9, 254, 255] {
        assert_eq!(
            HeadingReference::from_u8(unknown),
            HeadingReference::Unknown
        );
    }
}

#[test]
fn labels_identify_every_reference() {
    assert_eq!(HeadingReference::Magnetic.label(), "MAG");
    assert_eq!(HeadingReference::True.label(), "TRU");
    assert_eq!(HeadingReference::SimLocalTrue.label(), "SIM");
    assert_eq!(HeadingReference::Unknown.label(), "REF");
}
