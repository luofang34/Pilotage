//! A human takes the motion scope while the aircraft flies its holding
//! pattern after the plan: the mission is held and commands nothing, and
//! when the human hands the scope back the pattern is entered again from
//! where the aircraft is.

use pilotage_mission::{MissionEvent, MissionState};

use super::aircraft::STEP_S;
use super::{FIX_NED, Feed, Run};

#[test]
fn a_human_hold_in_the_pattern_is_held_and_resume_enters_the_pattern_again() {
    let mut run = Run::new(Feed::Navigator, [0.0, 0.0]);
    run.fly_and_hold(300.0);
    let started = |run: &Run| {
        run.events
            .iter()
            .filter(|event| matches!(event, MissionEvent::HoldingPatternStarted { .. }))
            .count()
    };
    assert_eq!(started(&run), 1);

    // The human flies it away for two minutes, on whatever heading it had.
    run.engine.hold(run.now);
    let held_from = run.samples.len();
    for _ in 0..(120.0 / STEP_S) as usize {
        run.step(true);
    }
    let held = run.samples.get(held_from..).unwrap_or_default();
    assert!(
        held.iter()
            .all(|sample| sample.state == MissionState::Held && !sample.commanded),
        "held, and nothing commanded"
    );
    assert!(
        run.events
            .iter()
            .any(|event| matches!(event, MissionEvent::MissionHeld))
    );

    // Handed back: the pattern is entered again and flown.
    assert!(run.engine.resume(run.now).is_some(), "the hold is resumed");
    let back_from = run.samples.len();
    for _ in 0..(900.0 / STEP_S) as usize {
        run.step(true);
    }
    let back = run.samples.get(back_from..).unwrap_or_default();
    assert!(
        back.iter()
            .all(|sample| sample.state == MissionState::AfterPlan && sample.commanded),
        "flying on after the plan, every tick commanded"
    );
    assert_eq!(started(&run), 2, "the pattern is entered again");
    assert!(
        back.last().is_some_and(|sample| sample.circuits >= 1),
        "and flown round"
    );
    let last = back.last().map_or([f64::NAN; 3], |sample| sample.ned);
    assert!(
        (last[0] - FIX_NED[0]).hypot(last[1] - FIX_NED[1]) < 8_000.0,
        "near the fix again: {last:?}"
    );
}
