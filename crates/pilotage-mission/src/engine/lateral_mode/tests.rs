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
    LateralMode::Coast,
];

const EVENTS: [LateralEvent; 11] = [
    LateralEvent::Disengaged,
    LateralEvent::Demand(Demand::Standby),
    LateralEvent::Demand(Demand::Position),
    LateralEvent::Demand(Demand::DirectTo),
    LateralEvent::Demand(Demand::Track),
    LateralEvent::Demand(Demand::Pattern),
    LateralEvent::Lost(LossResponse::HoldPosition),
    LateralEvent::Lost(LossResponse::Silent),
    LateralEvent::Lost(LossResponse::HoldHeading),
    LateralEvent::Lost(LossResponse::Coast),
    LateralEvent::TimerExpired,
];

/// Every (mode, event) pair with its stated outcome.
#[rustfmt::skip]
const TABLE: [(LateralMode, LateralEvent, LateralMode, LateralEffect); 99] = {
    use LateralEffect::{HeadingHoldBegan as HeadingBegan, None as Nothing, Resumed, TimedHoldBegan as TimedBegan};
    use LateralEvent::{Demand as D, Disengaged, Lost as L, TimerExpired as Expired};
    use LateralMode::{Coast, DirectTo, HeadingHold, Off, Pattern, PositionHold, Standby, TimedHold, Track};
    use LossResponse as R;
    [
        (Off, Disengaged, Off, Nothing),
        (Off, D(Demand::Standby), Standby, Nothing),
        (Off, D(Demand::Position), PositionHold, Nothing),
        (Off, D(Demand::DirectTo), DirectTo, Nothing),
        (Off, D(Demand::Track), Track, Nothing),
        (Off, D(Demand::Pattern), Pattern, Nothing),
        (Off, L(R::HoldPosition), PositionHold, Nothing),
        (Off, L(R::Silent), Off, Nothing),
        (Off, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (Off, L(R::Coast), Coast, Nothing),
        (Off, Expired, Off, Nothing),

        (Standby, Disengaged, Off, Nothing),
        (Standby, D(Demand::Standby), Standby, Nothing),
        (Standby, D(Demand::Position), PositionHold, Nothing),
        (Standby, D(Demand::DirectTo), DirectTo, Nothing),
        (Standby, D(Demand::Track), Track, Nothing),
        (Standby, D(Demand::Pattern), Pattern, Nothing),
        (Standby, L(R::HoldPosition), PositionHold, Nothing),
        (Standby, L(R::Silent), Off, Nothing),
        (Standby, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (Standby, L(R::Coast), Coast, Nothing),
        (Standby, Expired, Standby, Nothing),

        (PositionHold, Disengaged, Off, Nothing),
        (PositionHold, D(Demand::Standby), Standby, Nothing),
        (PositionHold, D(Demand::Position), PositionHold, Nothing),
        (PositionHold, D(Demand::DirectTo), DirectTo, Nothing),
        (PositionHold, D(Demand::Track), Track, Nothing),
        (PositionHold, D(Demand::Pattern), Pattern, Nothing),
        (PositionHold, L(R::HoldPosition), PositionHold, Nothing),
        (PositionHold, L(R::Silent), Off, Nothing),
        (PositionHold, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (PositionHold, L(R::Coast), Coast, Nothing),
        (PositionHold, Expired, PositionHold, Nothing),

        (DirectTo, Disengaged, Off, Nothing),
        (DirectTo, D(Demand::Standby), Standby, Nothing),
        (DirectTo, D(Demand::Position), PositionHold, Nothing),
        (DirectTo, D(Demand::DirectTo), DirectTo, Nothing),
        (DirectTo, D(Demand::Track), Track, Nothing),
        (DirectTo, D(Demand::Pattern), Pattern, Nothing),
        (DirectTo, L(R::HoldPosition), PositionHold, Nothing),
        (DirectTo, L(R::Silent), Off, Nothing),
        (DirectTo, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (DirectTo, L(R::Coast), Coast, Nothing),
        (DirectTo, Expired, DirectTo, Nothing),

        (Track, Disengaged, Off, Nothing),
        (Track, D(Demand::Standby), Standby, Nothing),
        (Track, D(Demand::Position), PositionHold, Nothing),
        (Track, D(Demand::DirectTo), DirectTo, Nothing),
        (Track, D(Demand::Track), Track, Nothing),
        (Track, D(Demand::Pattern), Pattern, Nothing),
        (Track, L(R::HoldPosition), PositionHold, Nothing),
        (Track, L(R::Silent), Off, Nothing),
        (Track, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (Track, L(R::Coast), Coast, Nothing),
        (Track, Expired, Track, Nothing),

        (Pattern, Disengaged, Off, Nothing),
        (Pattern, D(Demand::Standby), Standby, Nothing),
        (Pattern, D(Demand::Position), PositionHold, Nothing),
        (Pattern, D(Demand::DirectTo), DirectTo, Nothing),
        (Pattern, D(Demand::Track), Track, Nothing),
        (Pattern, D(Demand::Pattern), Pattern, Nothing),
        (Pattern, L(R::HoldPosition), PositionHold, Nothing),
        (Pattern, L(R::Silent), Off, Nothing),
        (Pattern, L(R::HoldHeading), TimedHold, TimedBegan),
        (Pattern, L(R::Coast), Coast, Nothing),
        (Pattern, Expired, Pattern, Nothing),

        (HeadingHold, Disengaged, Off, Nothing),
        (HeadingHold, D(Demand::Standby), Standby, Resumed),
        (HeadingHold, D(Demand::Position), PositionHold, Resumed),
        (HeadingHold, D(Demand::DirectTo), DirectTo, Resumed),
        (HeadingHold, D(Demand::Track), Track, Resumed),
        (HeadingHold, D(Demand::Pattern), Pattern, Resumed),
        (HeadingHold, L(R::HoldPosition), PositionHold, Nothing),
        (HeadingHold, L(R::Silent), Off, Nothing),
        (HeadingHold, L(R::HoldHeading), HeadingHold, Nothing),
        (HeadingHold, L(R::Coast), Coast, Nothing),
        (HeadingHold, Expired, TimedHold, TimedBegan),

        (TimedHold, Disengaged, Off, Nothing),
        (TimedHold, D(Demand::Standby), Standby, Resumed),
        (TimedHold, D(Demand::Position), PositionHold, Resumed),
        (TimedHold, D(Demand::DirectTo), DirectTo, Resumed),
        (TimedHold, D(Demand::Track), Track, Resumed),
        (TimedHold, D(Demand::Pattern), Pattern, Resumed),
        (TimedHold, L(R::HoldPosition), PositionHold, Nothing),
        (TimedHold, L(R::Silent), Off, Nothing),
        (TimedHold, L(R::HoldHeading), TimedHold, Nothing),
        (TimedHold, L(R::Coast), Coast, Nothing),
        (TimedHold, Expired, TimedHold, Nothing),

        (Coast, Disengaged, Off, Nothing),
        (Coast, D(Demand::Standby), Standby, Nothing),
        (Coast, D(Demand::Position), PositionHold, Nothing),
        (Coast, D(Demand::DirectTo), DirectTo, Nothing),
        (Coast, D(Demand::Track), Track, Nothing),
        (Coast, D(Demand::Pattern), Pattern, Nothing),
        (Coast, L(R::HoldPosition), PositionHold, Nothing),
        (Coast, L(R::Silent), Off, Nothing),
        (Coast, L(R::HoldHeading), HeadingHold, HeadingBegan),
        (Coast, L(R::Coast), Coast, Nothing),
        (Coast, Expired, Coast, Nothing),
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
    // Whatever it flew, a lost position answered by a heading hold ends
    // in the timed racetrack once the timer runs out, and stays there
    // while the position stays lost.
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
        ] {
            assert_eq!(
                next(held, LateralEvent::Demand(demand)),
                (mode, LateralEffect::Resumed),
                "{held:?} resumes {demand:?}"
            );
        }
    }
    // A demand that follows a demand resumes nothing.
    assert_eq!(
        next(LateralMode::DirectTo, LateralEvent::Demand(Demand::Track)).1,
        LateralEffect::None
    );
}
