use super::*;

const STATES: [NavState; 6] = [
    NavState::NotStarted,
    NavState::Fit,
    NavState::DeadReckoning,
    NavState::Lost(Loss::Stale),
    NavState::Lost(Loss::Unusable),
    NavState::Lost(Loss::TooUncertain),
];

const EVENTS: [NavEvent; 6] = [
    NavEvent::NothingHeld,
    NavEvent::Aged,
    NavEvent::WithinLimits,
    NavEvent::DeadReckoned,
    NavEvent::GradedUnusable,
    NavEvent::TooUncertain,
];

/// Every (state, event) pair with its stated outcome.
#[rustfmt::skip]
const TABLE: [(NavState, NavEvent, NavState, NavEffect); 36] = {
    use Loss::{Stale, TooUncertain as Uncertain, Unusable};
    use NavEffect as X;
    use NavEvent as E;
    use NavState::{DeadReckoning as Dr, Fit, Lost, NotStarted};
    [
        (NotStarted, E::NothingHeld, NotStarted, X::None),
        (NotStarted, E::Aged, Lost(Stale), X::Lost(Stale)),
        (NotStarted, E::WithinLimits, Fit, X::None),
        (NotStarted, E::DeadReckoned, Dr, X::DeadReckoningBegan),
        (NotStarted, E::GradedUnusable, Lost(Unusable), X::Lost(Unusable)),
        (NotStarted, E::TooUncertain, Lost(Uncertain), X::Lost(Uncertain)),

        (Fit, E::NothingHeld, Lost(Stale), X::Lost(Stale)),
        (Fit, E::Aged, Lost(Stale), X::Lost(Stale)),
        (Fit, E::WithinLimits, Fit, X::None),
        (Fit, E::DeadReckoned, Dr, X::DeadReckoningBegan),
        (Fit, E::GradedUnusable, Lost(Unusable), X::Lost(Unusable)),
        (Fit, E::TooUncertain, Lost(Uncertain), X::Lost(Uncertain)),

        (Dr, E::NothingHeld, Lost(Stale), X::Lost(Stale)),
        (Dr, E::Aged, Lost(Stale), X::Lost(Stale)),
        (Dr, E::WithinLimits, Fit, X::Recovered),
        (Dr, E::DeadReckoned, Dr, X::None),
        (Dr, E::GradedUnusable, Lost(Unusable), X::Lost(Unusable)),
        (Dr, E::TooUncertain, Lost(Uncertain), X::Lost(Uncertain)),

        (Lost(Stale), E::NothingHeld, Lost(Stale), X::None),
        (Lost(Stale), E::Aged, Lost(Stale), X::None),
        (Lost(Stale), E::WithinLimits, Fit, X::Recovered),
        (Lost(Stale), E::DeadReckoned, Dr, X::DeadReckoningBegan),
        (Lost(Stale), E::GradedUnusable, Lost(Unusable), X::Lost(Unusable)),
        (Lost(Stale), E::TooUncertain, Lost(Uncertain), X::Lost(Uncertain)),

        (Lost(Unusable), E::NothingHeld, Lost(Stale), X::Lost(Stale)),
        (Lost(Unusable), E::Aged, Lost(Stale), X::Lost(Stale)),
        (Lost(Unusable), E::WithinLimits, Fit, X::Recovered),
        (Lost(Unusable), E::DeadReckoned, Dr, X::DeadReckoningBegan),
        (Lost(Unusable), E::GradedUnusable, Lost(Unusable), X::None),
        (Lost(Unusable), E::TooUncertain, Lost(Uncertain), X::Lost(Uncertain)),

        (Lost(Uncertain), E::NothingHeld, Lost(Stale), X::Lost(Stale)),
        (Lost(Uncertain), E::Aged, Lost(Stale), X::Lost(Stale)),
        (Lost(Uncertain), E::WithinLimits, Fit, X::Recovered),
        (Lost(Uncertain), E::DeadReckoned, Dr, X::DeadReckoningBegan),
        (Lost(Uncertain), E::GradedUnusable, Lost(Unusable), X::Lost(Unusable)),
        (Lost(Uncertain), E::TooUncertain, Lost(Uncertain), X::None),
    ]
};

#[test]
fn every_state_and_event_has_its_stated_outcome() {
    for state in STATES {
        for event in EVENTS {
            let rows: Vec<_> = TABLE
                .iter()
                .filter(|(from, on, _, _)| *from == state && *on == event)
                .collect();
            assert_eq!(rows.len(), 1, "one row for {state:?} on {event:?}");
            let (_, _, to, effect) = *rows[0];
            assert_eq!(next(state, event), (to, effect), "{state:?} on {event:?}");
        }
    }
}

const fn held(fresh: bool, graded_unusable: bool, sigma: f64) -> Option<Held> {
    Some(Held {
        fresh,
        graded_unusable,
        horizontal_1sigma_m: sigma,
    })
}

const HOLD: Limits = Limits {
    max_horizontal_1sigma_m: 50.0,
    beyond: BeyondLimits::Hold,
};

const fn reckon(ceiling_1sigma_m: Option<f64>) -> Limits {
    Limits {
        max_horizontal_1sigma_m: 50.0,
        beyond: BeyondLimits::DeadReckon { ceiling_1sigma_m },
    }
}

/// The guards of [`classify`]: what is held, the limits, whether in
/// flight, and the event they choose.
#[rustfmt::skip]
const GUARDS: [(&str, Option<Held>, Limits, bool, NavEvent); 16] = [
    ("nothing held", None, HOLD, true, NavEvent::NothingHeld),
    ("old", held(false, false, 5.0), HOLD, true, NavEvent::Aged),
    ("old outranks unusable", held(false, true, 5.0), reckon(None), true, NavEvent::Aged),
    ("within", held(true, false, 5.0), HOLD, false, NavEvent::WithinLimits),
    ("at the limit", held(true, false, 50.0), HOLD, true, NavEvent::WithinLimits),
    ("over, hold", held(true, false, 51.0), HOLD, true, NavEvent::TooUncertain),
    ("unusable, hold", held(true, true, 5.0), HOLD, true, NavEvent::GradedUnusable),
    ("over, in flight", held(true, false, 51.0), reckon(None), true, NavEvent::DeadReckoned),
    ("unusable, in flight", held(true, true, 5e3), reckon(None), true, NavEvent::DeadReckoned),
    ("over, before flight", held(true, false, 51.0), reckon(None), false, NavEvent::TooUncertain),
    ("unusable, before flight", held(true, true, 5.0), reckon(None), false, NavEvent::GradedUnusable),
    ("at the ceiling", held(true, false, 500.0), reckon(Some(500.0)), true, NavEvent::DeadReckoned),
    ("past the ceiling", held(true, false, 501.0), reckon(Some(500.0)), true, NavEvent::TooUncertain),
    ("unusable past it", held(true, true, 501.0), reckon(Some(500.0)), true, NavEvent::GradedUnusable),
    ("unknown uncertainty", held(true, false, f64::NAN), reckon(None), true, NavEvent::TooUncertain),
    ("infinite uncertainty", held(true, false, f64::INFINITY), reckon(None), true, NavEvent::TooUncertain),
];

#[test]
fn the_guards_choose_the_event() {
    for (name, held, limits, in_flight, event) in GUARDS {
        assert_eq!(classify(held, limits, in_flight), event, "{name}");
    }
}

#[test]
fn only_a_fit_or_dead_reckoned_source_steers() {
    for state in STATES {
        assert_eq!(
            state.steers(),
            matches!(state, NavState::Fit | NavState::DeadReckoning),
            "{state:?}"
        );
    }
}
