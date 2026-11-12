use navigate_contract::{SensorClass, SourceComposition};

use super::{
    PositionIntegrity, PositionKind, PositionSource, SourceIntegrity, SourceIntegrityState,
    SteeringIntegrity, SteeringSource, kind_of,
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

/// The navigator's statement of its satellite receiver in `state`.
const fn gnss(state: SourceIntegrityState) -> Option<SourceIntegrity> {
    Some(SourceIntegrity {
        class: SensorClass::Gnss,
        state,
        since: None,
    })
}

const GOOD: SteeringIntegrity = SteeringIntegrity::WithinLimits {
    quality: NavQuality::Good,
    horizontal_1sigma_m: 3.0,
};
const DEGRADED: SteeringIntegrity = SteeringIntegrity::WithinLimits {
    quality: NavQuality::Degraded,
    horizontal_1sigma_m: 30.0,
};
const UNUSABLE: SteeringIntegrity = SteeringIntegrity::WithinLimits {
    quality: NavQuality::Unusable,
    horizontal_1sigma_m: 30.0,
};
const RECKONED: SteeringIntegrity = SteeringIntegrity::DeadReckoning {
    quality: NavQuality::Unusable,
    horizontal_1sigma_m: 900.0,
};
const NONE: SteeringIntegrity = SteeringIntegrity::NoSolution {
    reason: StartWithheldReason::NavigationStale,
};

/// Every judgment with every statement of the navigator, and what an
/// instrument annunciates.
#[rustfmt::skip]
const ANNUNCIATED: [(SteeringIntegrity, Option<SourceIntegrityState>, Option<PositionIntegrity>); 20] = {
    use PositionIntegrity::{Degraded, Normal, SensorExcluded};
    use SourceIntegrityState::{Consistent, Disagreeing, Excluded};
    [
        (GOOD, None, Some(Normal)),
        (GOOD, Some(Consistent), Some(Normal)),
        (GOOD, Some(Disagreeing), Some(Degraded)),
        (GOOD, Some(Excluded), Some(SensorExcluded)),
        (DEGRADED, None, Some(Degraded)),
        (DEGRADED, Some(Consistent), Some(Degraded)),
        (DEGRADED, Some(Disagreeing), Some(Degraded)),
        (DEGRADED, Some(Excluded), Some(SensorExcluded)),
        (RECKONED, None, Some(Degraded)),
        (RECKONED, Some(Consistent), Some(Degraded)),
        (RECKONED, Some(Disagreeing), Some(Degraded)),
        (RECKONED, Some(Excluded), Some(SensorExcluded)),
        (UNUSABLE, None, None),
        (UNUSABLE, Some(Consistent), None),
        (UNUSABLE, Some(Disagreeing), None),
        (UNUSABLE, Some(Excluded), None),
        (NONE, None, None),
        (NONE, Some(Consistent), None),
        (NONE, Some(Disagreeing), None),
        (NONE, Some(Excluded), None),
    ]
};

#[test]
fn a_sensor_the_navigator_excludes_or_doubts_is_annunciated() {
    for (integrity, state, expected) in ANNUNCIATED {
        let steering = SteeringSource::new(PositionSource::OnboardNavigation, None, integrity)
            .with_source_integrity(state.and_then(gnss));
        assert_eq!(
            steering.position_integrity(),
            expected,
            "{integrity:?} with {state:?}"
        );
    }
}
