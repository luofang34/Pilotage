#![allow(clippy::expect_used, clippy::panic)]

use super::*;

/// A representative condition for every known identity.
const CATALOG: &[AlertCondition] = &[
    AlertCondition::Altitude(AltFault::ReferenceLost),
    AlertCondition::Altitude(AltFault::DatumMiscompare),
    AlertCondition::Altitude(AltFault::Unavailable),
    AlertCondition::Heading(NavFault::HeadingReferenceLost),
    AlertCondition::Heading(NavFault::CourseSourceInvalid),
    AlertCondition::Heading(NavFault::Unavailable),
    AlertCondition::TurnSlip(DynFault::TurnRateInvalid),
    AlertCondition::TurnSlip(DynFault::SlipInvalid),
    AlertCondition::TurnSlip(DynFault::Unavailable),
    AlertCondition::Miscompare(MiscompareFault::Attitude),
    AlertCondition::Miscompare(MiscompareFault::Airspeed),
    AlertCondition::Miscompare(MiscompareFault::Altitude),
    AlertCondition::Miscompare(MiscompareFault::Heading),
    AlertCondition::Display(DisplayFault::RendererStalled),
    AlertCondition::Display(DisplayFault::FrameGenerationLost),
    AlertCondition::Display(DisplayFault::CommandBufferCorrupt),
    AlertCondition::Display(DisplayFault::BackendLost),
    AlertCondition::Display(DisplayFault::RetainedImage),
    AlertCondition::FrameMismatch { code: 7 },
    AlertCondition::System(SystemNote::DatabaseStale),
    AlertCondition::System(SystemNote::MaintenanceRequired),
    AlertCondition::System(SystemNote::ConfigMismatch),
];

#[test]
fn identities_are_unique() {
    for (i, a) in CATALOG.iter().enumerate() {
        for b in &CATALOG[i + 1..] {
            assert_ne!(a.id(), b.id(), "collision between {a:?} and {b:?}");
        }
    }
}

#[test]
fn class_of_agrees_with_condition_class() {
    for cond in CATALOG {
        assert_eq!(
            class_of(cond.id()),
            Some(cond.class()),
            "class mismatch for {cond:?}"
        );
    }
}

#[test]
fn every_frame_code_resolves_to_caution() {
    for code in 0..=u8::MAX {
        let cond = AlertCondition::FrameMismatch { code };
        assert_eq!(cond.class(), AlertClass::Caution);
        assert_eq!(class_of(cond.id()), Some(AlertClass::Caution));
    }
}

#[test]
fn unknown_identities_resolve_to_none() {
    // Unused code inside a known family, and an unknown family.
    assert_eq!(class_of(AlertId(0x0109)), None);
    assert_eq!(class_of(AlertId(0x0900)), None);
}

#[test]
fn attitude_miscompare_is_a_warning() {
    assert_eq!(
        AlertCondition::Miscompare(MiscompareFault::Attitude).class(),
        AlertClass::Warning
    );
}
