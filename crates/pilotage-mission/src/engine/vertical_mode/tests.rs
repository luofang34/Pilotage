use super::*;

const MODES: [VerticalMode; 5] = [
    VerticalMode::Off,
    VerticalMode::Pending,
    VerticalMode::Climb,
    VerticalMode::Descend,
    VerticalMode::Hold,
];

const RELATIONS: [Relation; 4] = [
    Relation::Below,
    Relation::Within,
    Relation::Above,
    Relation::Unknown,
];

/// Every (mode, event) pair with its stated outcome.
#[rustfmt::skip]
const TABLE: [(VerticalMode, VerticalEvent, VerticalMode, VerticalEffect); 45] = {
    use Relation::{Above, Below, Unknown, Within};
    use VerticalEffect::{Captured, None as Nothing};
    use VerticalEvent::{Disengaged, Sample, TargetSet};
    use VerticalMode::{Climb, Descend, Hold, Off, Pending};
    [
        (Off, Disengaged, Off, Nothing),
        (Off, TargetSet(Below), Climb, Nothing),
        (Off, TargetSet(Within), Hold, Captured),
        (Off, TargetSet(Above), Descend, Nothing),
        (Off, TargetSet(Unknown), Pending, Nothing),
        (Off, Sample(Below), Off, Nothing),
        (Off, Sample(Within), Off, Nothing),
        (Off, Sample(Above), Off, Nothing),
        (Off, Sample(Unknown), Off, Nothing),

        (Pending, Disengaged, Off, Nothing),
        (Pending, TargetSet(Below), Climb, Nothing),
        (Pending, TargetSet(Within), Hold, Captured),
        (Pending, TargetSet(Above), Descend, Nothing),
        (Pending, TargetSet(Unknown), Pending, Nothing),
        (Pending, Sample(Below), Climb, Nothing),
        (Pending, Sample(Within), Hold, Captured),
        (Pending, Sample(Above), Descend, Nothing),
        (Pending, Sample(Unknown), Pending, Nothing),

        (Climb, Disengaged, Off, Nothing),
        (Climb, TargetSet(Below), Climb, Nothing),
        (Climb, TargetSet(Within), Hold, Captured),
        (Climb, TargetSet(Above), Descend, Nothing),
        (Climb, TargetSet(Unknown), Pending, Nothing),
        (Climb, Sample(Below), Climb, Nothing),
        (Climb, Sample(Within), Hold, Captured),
        (Climb, Sample(Above), Hold, Captured),
        (Climb, Sample(Unknown), Climb, Nothing),

        (Descend, Disengaged, Off, Nothing),
        (Descend, TargetSet(Below), Climb, Nothing),
        (Descend, TargetSet(Within), Hold, Captured),
        (Descend, TargetSet(Above), Descend, Nothing),
        (Descend, TargetSet(Unknown), Pending, Nothing),
        (Descend, Sample(Below), Hold, Captured),
        (Descend, Sample(Within), Hold, Captured),
        (Descend, Sample(Above), Descend, Nothing),
        (Descend, Sample(Unknown), Descend, Nothing),

        (Hold, Disengaged, Off, Nothing),
        (Hold, TargetSet(Below), Climb, Nothing),
        (Hold, TargetSet(Within), Hold, Nothing),
        (Hold, TargetSet(Above), Descend, Nothing),
        (Hold, TargetSet(Unknown), Pending, Nothing),
        (Hold, Sample(Below), Hold, Nothing),
        (Hold, Sample(Within), Hold, Nothing),
        (Hold, Sample(Above), Hold, Nothing),
        (Hold, Sample(Unknown), Hold, Nothing),
    ]
};

fn events() -> Vec<VerticalEvent> {
    let mut events = vec![VerticalEvent::Disengaged];
    events.extend(RELATIONS.map(VerticalEvent::TargetSet));
    events.extend(RELATIONS.map(VerticalEvent::Sample));
    events
}

#[test]
fn every_mode_and_event_has_its_stated_outcome() {
    for mode in MODES {
        for event in events() {
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

/// The guard: an altitude, a target, a margin, and where that puts the
/// vehicle.
#[rustfmt::skip]
const RELATED: [(&str, Option<f64>, f64, f64, Relation); 9] = [
    ("far below", Some(1_700.0), 2_000.0, 30.0, Relation::Below),
    ("just outside below", Some(1_969.9), 2_000.0, 30.0, Relation::Below),
    ("at the margin below", Some(1_970.0), 2_000.0, 30.0, Relation::Within),
    ("on target", Some(2_000.0), 2_000.0, 1.0, Relation::Within),
    ("at the margin above", Some(2_001.0), 2_000.0, 1.0, Relation::Within),
    ("just outside above", Some(2_001.1), 2_000.0, 1.0, Relation::Above),
    ("no altitude", None, 2_000.0, 1.0, Relation::Unknown),
    ("an altitude that is not a number", Some(f64::NAN), 2_000.0, 1.0, Relation::Unknown),
    ("a target that is not a number", Some(2_000.0), f64::NAN, 1.0, Relation::Unknown),
];

#[test]
fn the_margin_relates_the_altitude_to_the_target() {
    for (name, altitude_m, target_m, margin_m, relation) in RELATED {
        assert_eq!(relate(altitude_m, target_m, margin_m), relation, "{name}");
    }
}
