use navigate_contract::{SensorClass, SourceComposition};

use super::{
    PositionIntegrity, PositionKind, PositionSource, SteeringIntegrity, SteeringSource, kind_of,
};
use crate::engine::{NavQuality, StartWithheldReason};

fn of(classes: &[SensorClass]) -> SourceComposition {
    classes
        .iter()
        .fold(SourceComposition::empty(), |set, class| set.with(*class))
}

#[test]
fn the_position_aids_name_the_kind() {
    use SensorClass::{
        Barometric, Celestial, FcState, Gnss, Inertial, NavigationSolution, RadioNavigation,
        SurfaceAltimeter, VisualLandmark, VisualOdometry,
    };
    let cases: [(&[SensorClass], PositionKind); 11] = [
        (&[Inertial, Barometric], PositionKind::Inertial),
        (&[Inertial, SurfaceAltimeter], PositionKind::Inertial),
        (&[Gnss], PositionKind::Satellite),
        (&[Gnss, Inertial, Barometric], PositionKind::Satellite),
        (&[VisualLandmark, Inertial], PositionKind::Visual),
        (&[VisualOdometry, VisualLandmark], PositionKind::Visual),
        (&[Gnss, VisualLandmark, Inertial], PositionKind::Blended),
        (&[Gnss, RadioNavigation], PositionKind::Blended),
        (&[Celestial, Inertial], PositionKind::Blended),
        (&[FcState], PositionKind::Blended),
        (&[NavigationSolution], PositionKind::Blended),
    ];
    for (classes, kind) in cases {
        assert_eq!(kind_of(of(classes)), kind, "{classes:?}");
    }
}

#[test]
fn the_judgment_outranks_the_aids() {
    let gnss = Some(SourceComposition::of(SensorClass::Gnss));
    let within = SteeringIntegrity::WithinLimits {
        quality: NavQuality::Good,
        horizontal_1sigma_m: 3.0,
    };
    let reckoned = SteeringIntegrity::DeadReckoning {
        quality: NavQuality::Unusable,
        horizontal_1sigma_m: 900.0,
    };
    let none = SteeringIntegrity::NoSolution {
        reason: StartWithheldReason::NavigationStale,
    };
    let kind = |contributing, integrity| {
        SteeringSource::new(PositionSource::OnboardNavigation, contributing, integrity)
            .position_kind()
    };
    assert_eq!(kind(gnss, within), Some(PositionKind::Satellite));
    assert_eq!(kind(gnss, reckoned), Some(PositionKind::DeadReckoning));
    assert_eq!(kind(gnss, none), None);
    assert_eq!(
        kind(None, within),
        Some(PositionKind::Blended),
        "a solution that names no sensor, as a simulator's truth"
    );
}

#[test]
fn the_judgment_and_the_grade_give_the_annunciated_integrity() {
    let within = |quality| SteeringIntegrity::WithinLimits {
        quality,
        horizontal_1sigma_m: 3.0,
    };
    let reckoned = |quality| SteeringIntegrity::DeadReckoning {
        quality,
        horizontal_1sigma_m: 900.0,
    };
    let cases = [
        (within(NavQuality::Good), Some(PositionIntegrity::Normal)),
        (
            within(NavQuality::Degraded),
            Some(PositionIntegrity::Degraded),
        ),
        (within(NavQuality::Unusable), None),
        (
            reckoned(NavQuality::Good),
            Some(PositionIntegrity::Degraded),
        ),
        (
            reckoned(NavQuality::Degraded),
            Some(PositionIntegrity::Degraded),
        ),
        (
            reckoned(NavQuality::Unusable),
            Some(PositionIntegrity::Degraded),
        ),
        (
            SteeringIntegrity::NoSolution {
                reason: StartWithheldReason::NavigationStale,
            },
            None,
        ),
    ];
    for (integrity, expected) in cases {
        let steering = SteeringSource::new(PositionSource::OnboardNavigation, None, integrity);
        assert_eq!(steering.position_integrity(), expected, "{integrity:?}");
    }
}
