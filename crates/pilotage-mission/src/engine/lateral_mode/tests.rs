use super::*;

const MODES: [LateralMode; 9] = [
    LateralMode::Off,
    LateralMode::Standby,
    LateralMode::PositionHold,
    LateralMode::DirectTo,
    LateralMode::Track,
    LateralMode::Pattern,
    LateralMode::HeadingHold,
    LateralMode::TimedHold,
    LateralMode::HeadingSelect,
];

const EVENTS: [LateralEvent; 11] = [
    LateralEvent::Disengaged,
    LateralEvent::Demand(Demand::Standby),
    LateralEvent::Demand(Demand::Position),
    LateralEvent::Demand(Demand::DirectTo),
    LateralEvent::Demand(Demand::Track),
    LateralEvent::Demand(Demand::Pattern),
    LateralEvent::Demand(Demand::Heading),
    LateralEvent::Lost(LossResponse::HoldPosition),
    LateralEvent::Lost(LossResponse::Silent),
    LateralEvent::Lost(LossResponse::HoldHeading),
    LateralEvent::TimerExpired,
];

/// Every (mode, event) pair with its stated outcome.
#[rustfmt::skip]
const TABLE: [(LateralMode, LateralEvent, LateralMode, LateralEffect); 99] = {
    use LateralEffect::{HeadingHoldBegan as HeadingBegan, None as Nothing, Resumed, TimedHoldBegan as TimedBegan};
    use LateralEvent::{Demand as D, Disengaged, Lost as L, TimerExpired as Expired};
    use LateralMode::{DirectTo, HeadingHold, HeadingSelect, Off, Pattern, PositionHold, Standby, TimedHold, Track};
    use LossResponse as R;
    [
        (Off, Disengaged, Off, Nothing),
        (Off, D(Demand::Standby), Standby, Nothing),
        (Off, D(Demand::Position), PositionHold, Nothing),
        (Off, D(Demand::DirectTo), DirectTo, Nothing),
        (Off, D(Demand::Track), Track, Nothing),
        (Off, D(Demand::Pattern), Pattern, Nothing),
        (Off, D(Demand::Heading), HeadingSelect, Nothing),
        (Off, L(R::HoldPosition), PositionHold, Nothing),
        (Off, L(R::Silent), Off, Nothing),
        (Off, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (Off, Expired, Off, Nothing),

        (Standby, Disengaged, Off, Nothing),
        (Standby, D(Demand::Standby), Standby, Nothing),
        (Standby, D(Demand::Position), PositionHold, Nothing),
        (Standby, D(Demand::DirectTo), DirectTo, Nothing),
        (Standby, D(Demand::Track), Track, Nothing),
        (Standby, D(Demand::Pattern), Pattern, Nothing),
        (Standby, D(Demand::Heading), HeadingSelect, Nothing),
        (Standby, L(R::HoldPosition), PositionHold, Nothing),
        (Standby, L(R::Silent), Off, Nothing),
        (Standby, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (Standby, Expired, Standby, Nothing),

        (PositionHold, Disengaged, Off, Nothing),
        (PositionHold, D(Demand::Standby), Standby, Nothing),
        (PositionHold, D(Demand::Position), PositionHold, Nothing),
        (PositionHold, D(Demand::DirectTo), DirectTo, Nothing),
        (PositionHold, D(Demand::Track), Track, Nothing),
        (PositionHold, D(Demand::Pattern), Pattern, Nothing),
        (PositionHold, D(Demand::Heading), HeadingSelect, Nothing),
        (PositionHold, L(R::HoldPosition), PositionHold, Nothing),
        (PositionHold, L(R::Silent), Off, Nothing),
        (PositionHold, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (PositionHold, Expired, PositionHold, Nothing),

        (DirectTo, Disengaged, Off, Nothing),
        (DirectTo, D(Demand::Standby), Standby, Nothing),
        (DirectTo, D(Demand::Position), PositionHold, Nothing),
        (DirectTo, D(Demand::DirectTo), DirectTo, Nothing),
        (DirectTo, D(Demand::Track), Track, Nothing),
        (DirectTo, D(Demand::Pattern), Pattern, Nothing),
        (DirectTo, D(Demand::Heading), HeadingSelect, Nothing),
        (DirectTo, L(R::HoldPosition), PositionHold, Nothing),
        (DirectTo, L(R::Silent), Off, Nothing),
        (DirectTo, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (DirectTo, Expired, DirectTo, Nothing),

        (Track, Disengaged, Off, Nothing),
        (Track, D(Demand::Standby), Standby, Nothing),
        (Track, D(Demand::Position), PositionHold, Nothing),
        (Track, D(Demand::DirectTo), DirectTo, Nothing),
        (Track, D(Demand::Track), Track, Nothing),
        (Track, D(Demand::Pattern), Pattern, Nothing),
        (Track, D(Demand::Heading), HeadingSelect, Nothing),
        (Track, L(R::HoldPosition), PositionHold, Nothing),
        (Track, L(R::Silent), Off, Nothing),
        (Track, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (Track, Expired, Track, Nothing),

        (Pattern, Disengaged, Off, Nothing),
        (Pattern, D(Demand::Standby), Standby, Nothing),
        (Pattern, D(Demand::Position), PositionHold, Nothing),
        (Pattern, D(Demand::DirectTo), DirectTo, Nothing),
        (Pattern, D(Demand::Track), Track, Nothing),
        (Pattern, D(Demand::Pattern), Pattern, Nothing),
        (Pattern, D(Demand::Heading), HeadingSelect, Nothing),
        (Pattern, L(R::HoldPosition), PositionHold, Nothing),
        (Pattern, L(R::Silent), Off, Nothing),
        (Pattern, L(R::HoldHeading), TimedHold, TimedBegan),
        (Pattern, Expired, Pattern, Nothing),

        (HeadingHold, Disengaged, Off, Nothing),
        (HeadingHold, D(Demand::Standby), Standby, Resumed),
        (HeadingHold, D(Demand::Position), PositionHold, Resumed),
        (HeadingHold, D(Demand::DirectTo), DirectTo, Resumed),
        (HeadingHold, D(Demand::Track), Track, Resumed),
        (HeadingHold, D(Demand::Pattern), Pattern, Resumed),
        (HeadingHold, D(Demand::Heading), HeadingSelect, Resumed),
        (HeadingHold, L(R::HoldPosition), PositionHold, Nothing),
        (HeadingHold, L(R::Silent), Off, Nothing),
        (HeadingHold, L(R::HoldHeading), HeadingHold, Nothing),
        (HeadingHold, Expired, TimedHold, TimedBegan),

        (TimedHold, Disengaged, Off, Nothing),
        (TimedHold, D(Demand::Standby), Standby, Resumed),
        (TimedHold, D(Demand::Position), PositionHold, Resumed),
        (TimedHold, D(Demand::DirectTo), DirectTo, Resumed),
        (TimedHold, D(Demand::Track), Track, Resumed),
        (TimedHold, D(Demand::Pattern), Pattern, Resumed),
        (TimedHold, D(Demand::Heading), HeadingSelect, Resumed),
        (TimedHold, L(R::HoldPosition), PositionHold, Nothing),
        (TimedHold, L(R::Silent), Off, Nothing),
        (TimedHold, L(R::HoldHeading), TimedHold, Nothing),
        (TimedHold, Expired, TimedHold, Nothing),

        (HeadingSelect, Disengaged, Off, Nothing),
        (HeadingSelect, D(Demand::Standby), Standby, Nothing),
        (HeadingSelect, D(Demand::Position), PositionHold, Nothing),
        (HeadingSelect, D(Demand::DirectTo), DirectTo, Nothing),
        (HeadingSelect, D(Demand::Track), Track, Nothing),
        (HeadingSelect, D(Demand::Pattern), Pattern, Nothing),
        (HeadingSelect, D(Demand::Heading), HeadingSelect, Nothing),
        (HeadingSelect, L(R::HoldPosition), PositionHold, Nothing),
        (HeadingSelect, L(R::Silent), Off, Nothing),
        (HeadingSelect, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (HeadingSelect, Expired, HeadingSelect, Nothing),
    ]
};

#[test]
fn every_mode_and_event_has_its_stated_outcome() {
    for mode in MODES {
        for event in EVENTS {
            let rows: Vec<_> = TABLE
                .iter()
                .filter(|(from, on, _, _)| *from == mode && *on == event)
                .collect();
            assert_eq!(rows.len(), 1, "one row for {mode:?} on {event:?}");
            let (_, _, to, effect) = *rows[0];
            assert_eq!(next(mode, event), (to, effect), "{mode:?} on {event:?}");
        }
    }
}

#[test]
fn a_vehicle_that_cannot_stop_never_holds_a_heading_without_end() {
    let lost = LateralEvent::Lost(LossResponse::HoldHeading);
    for mode in MODES {
        let (held, _) = next(mode, lost);
        let (expired, _) = next(held, LateralEvent::TimerExpired);
        assert_eq!(expired, LateralMode::TimedHold, "from {mode:?}");
        assert_eq!(next(expired, lost).0, LateralMode::TimedHold);
        assert_eq!(
            next(expired, LateralEvent::TimerExpired).0,
            LateralMode::TimedHold
        );
    }
}

#[test]
fn the_heading_hold_announces_its_start_once() {
    let lost = LateralEvent::Lost(LossResponse::HoldHeading);
    let (mode, first) = next(LateralMode::Track, lost);
    assert_eq!(first, LateralEffect::HeadingHoldBegan);
    assert_eq!(
        next(mode, lost),
        (LateralMode::HeadingHold, LateralEffect::None)
    );
}

#[test]
fn a_position_that_returns_resumes_the_mode_demanded_and_says_so() {
    for held in [LateralMode::HeadingHold, LateralMode::TimedHold] {
        for (demand, mode) in [
            (Demand::Track, LateralMode::Track),
            (Demand::DirectTo, LateralMode::DirectTo),
            (Demand::Pattern, LateralMode::Pattern),
            (Demand::Heading, LateralMode::HeadingSelect),
        ] {
            assert_eq!(
                next(held, LateralEvent::Demand(demand)),
                (mode, LateralEffect::Resumed),
                "{held:?} resumes {demand:?}"
            );
        }
    }
    assert_eq!(
        next(LateralMode::DirectTo, LateralEvent::Demand(Demand::Track)).1,
        LateralEffect::None
    );
}

#[test]
fn a_selected_heading_is_flown_from_every_mode_and_gives_way_to_a_lost_position() {
    for mode in MODES {
        assert_eq!(
            next(mode, LateralEvent::Demand(Demand::Heading)).0,
            LateralMode::HeadingSelect,
            "from {mode:?}"
        );
    }
    // A lost position wins over the selection: the bounded heading hold
    // takes over.
    assert_eq!(
        next(
            LateralMode::HeadingSelect,
            LateralEvent::Lost(LossResponse::HoldHeading)
        ),
        (LateralMode::HeadingHold, LateralEffect::HeadingHoldBegan)
    );
    // The selection cleared: the plan's demand is flown again.
    assert_eq!(
        next(
            LateralMode::HeadingSelect,
            LateralEvent::Demand(Demand::Track)
        ),
        (LateralMode::Track, LateralEffect::None)
    );
}
