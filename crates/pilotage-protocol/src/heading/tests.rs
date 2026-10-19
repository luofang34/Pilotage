use super::{HeadingReference, MagneticVariationSource};
use crate::wire;

const SOURCES: [MagneticVariationSource; 6] = [
    MagneticVariationSource::FcModel,
    MagneticVariationSource::FcConfigured,
    MagneticVariationSource::SharedModel,
    MagneticVariationSource::PublishedChart,
    MagneticVariationSource::Operator,
    MagneticVariationSource::None,
];

#[test]
fn every_heading_reference_round_trips_through_the_wire() {
    for reference in [HeadingReference::True, HeadingReference::Magnetic] {
        assert_eq!(
            HeadingReference::from_wire(reference.to_wire() as i32),
            Some(reference)
        );
        assert_ne!(
            reference.to_wire() as i32,
            0,
            "{reference:?} is not the default"
        );
    }
    assert_eq!(
        HeadingReference::option_to_wire(None),
        wire::HeadingReference::Unspecified
    );
}

#[test]
fn an_unspecified_or_unknown_heading_reference_is_none() {
    assert_eq!(
        HeadingReference::from_wire(wire::HeadingReference::Unspecified as i32),
        None
    );
    assert_eq!(HeadingReference::from_wire(42), None);
}

#[test]
fn every_variation_source_round_trips_through_the_wire() {
    for source in SOURCES {
        assert_eq!(
            MagneticVariationSource::from_wire(source.to_wire() as i32),
            Some(source)
        );
        assert_ne!(source.to_wire() as i32, 0, "{source:?} is not the default");
    }
    assert_eq!(
        MagneticVariationSource::from_wire(wire::MagneticVariationSource::Unspecified as i32),
        None
    );
    assert_eq!(MagneticVariationSource::from_wire(42), None);
}

#[test]
fn only_an_applied_variation_makes_a_true_heading() {
    for source in SOURCES {
        let expected = if source == MagneticVariationSource::None {
            HeadingReference::Magnetic
        } else {
            HeadingReference::True
        };
        assert_eq!(source.heading_reference(), expected, "{source:?}");
    }
}
