//! Grounding: what the operator said, and what a model made up.

#![allow(clippy::panic)]

use super::check_grounding;
use crate::directive::{Arrival, Directive, HoldPoint, TurnDirection};

fn direct(fix: &str) -> Directive {
    Directive::DirectTo {
        fix: fix.to_owned(),
        on_arrival: Arrival::Land,
    }
}

fn heading(degrees: u16) -> Directive {
    turn(degrees, TurnDirection::Left)
}

fn turn(degrees: u16, turn: TurnDirection) -> Directive {
    Directive::Heading { degrees, turn }
}

fn procedure(name: &str) -> Directive {
    Directive::JoinProcedure {
        procedure: name.to_owned(),
    }
}

#[test]
fn a_permitted_fix_that_the_operator_did_not_say_is_refused() {
    // A model gave ALPHA for a fix that does not exist. ALPHA is permitted,
    // so the envelope check passes it.
    let message = "Proceed direct ZULU and land.";
    assert!(check_grounding(message, &direct("ALPHA")).is_err());
    assert!(check_grounding("Proceed direct ALPHA and land.", &direct("ALPHA")).is_ok());
    assert!(check_grounding("direct alpha, land", &direct("ALPHA")).is_ok());
}

#[test]
fn a_name_inside_a_longer_word_does_not_count() {
    assert!(check_grounding("Fly the alphabet route.", &direct("ALPHA")).is_err());
}

#[test]
fn home_is_named_by_home_or_base() {
    assert!(check_grounding("Come back home and land.", &direct("HOME")).is_ok());
    assert!(check_grounding("Return to base.", &direct("HOME")).is_ok());
    assert!(check_grounding("Go to BRAVO.", &direct("HOME")).is_err());
}

#[test]
fn a_heading_must_be_the_number_that_was_said() {
    assert!(check_grounding("Turn left heading 270.", &heading(270)).is_ok());
    assert!(
        check_grounding("Turn left heading 270.", &heading(220)).is_err(),
        "a misread digit"
    );
    assert!(check_grounding("Turn left heading two seven zero.", &heading(270)).is_ok());
    assert!(check_grounding("Turn left heading three six zero.", &heading(0)).is_ok());
    assert!(check_grounding("Fly heading 045.", &turn(45, TurnDirection::Shortest)).is_ok());
    let right = turn(90, TurnDirection::Right);
    assert!(check_grounding("Come right to heading niner zero.", &right).is_ok());
    assert!(
        check_grounding("Turn left.", &heading(270)).is_err(),
        "no number was said"
    );
}

#[test]
fn a_height_and_a_speed_must_be_the_numbers_that_were_said() {
    let climb = Directive::Altitude { height_m: 20.0 };
    assert!(check_grounding("Climb and maintain 20 metres.", &climb).is_ok());
    assert!(check_grounding("Climb and maintain 12 metres.", &climb).is_err());
    let slow = Directive::Speed { speed_mps: 0.5 };
    assert!(check_grounding("Slow down to 0.5 metres per second.", &slow).is_ok());
    assert!(check_grounding("Slow down to 5 metres per second.", &slow).is_err());
}

#[test]
fn a_procedure_can_be_said_in_parts() {
    assert!(check_grounding("Cleared RNAV 27 approach.", &procedure("RNAV27")).is_ok());
    assert!(check_grounding("Join the RNAV two seven approach.", &procedure("RNAV27")).is_ok());
    assert!(
        check_grounding(
            "Cleared for the ILS runway 09 approach.",
            &procedure("ILS09")
        )
        .is_ok()
    );
    assert!(check_grounding("Fly the ILS09 procedure.", &procedure("ILS09")).is_ok());
    assert!(check_grounding("Cleared ILS 09 approach.", &procedure("RNAV27")).is_err());
    assert!(
        check_grounding("Cleared RNAV 09 approach.", &procedure("RNAV27")).is_err(),
        "the number is wrong"
    );
}

#[test]
fn a_directive_with_no_slot_still_needs_its_word() {
    let hold = Directive::Hold {
        point: HoldPoint::PresentPosition,
    };
    let cases = [
        (
            Directive::Takeoff {},
            "Cleared for takeoff.",
            "Get airborne.",
        ),
        (Directive::Land {}, "Cleared to land.", "Land now."),
        (
            Directive::GoAround {},
            "Go around.",
            "Go around, go around.",
        ),
        (
            Directive::ReturnToBase {},
            "Return to base.",
            "Come back home and land.",
        ),
        (hold, "Hold position.", "Hold present position."),
    ];
    for (directive, said, also_said) in cases {
        assert!(check_grounding(said, &directive).is_ok(), "{said}");
        assert!(
            check_grounding(also_said, &directive).is_ok(),
            "{also_said}"
        );
        assert!(
            check_grounding("Proceed direct BRAVO.", &directive).is_err(),
            "{directive:?} was not asked for"
        );
    }
}

#[test]
fn the_arrival_must_be_the_one_that_was_said() {
    let land = direct("BRAVO");
    let hold = Directive::DirectTo {
        fix: "BRAVO".to_owned(),
        on_arrival: Arrival::Hold,
    };
    assert!(check_grounding("Proceed direct BRAVO and hold.", &hold).is_ok());
    assert!(
        check_grounding("Proceed direct BRAVO.", &hold).is_ok(),
        "no landing asked"
    );
    assert!(
        check_grounding("Proceed direct BRAVO and hold.", &land).is_err(),
        "a landing nobody asked for"
    );
    assert!(check_grounding("Proceed direct BRAVO and land.", &land).is_ok());
    assert!(
        check_grounding("Proceed direct BRAVO and land.", &hold).is_err(),
        "a lost landing"
    );
}

#[test]
fn a_number_bound_to_a_heading_is_not_a_height() {
    let climb = Directive::Altitude { height_m: 12.0 };
    let text = "Climb to 12 metres and turn left heading 270.";
    assert!(check_grounding(text, &climb).is_ok());
    assert!(check_grounding(text, &heading(270)).is_ok());
    assert!(
        check_grounding(text, &heading(12)).is_err(),
        "12 is a height"
    );
    let low = Directive::Altitude { height_m: 270.0 };
    assert!(check_grounding(text, &low).is_err(), "270 is a heading");
    assert!(
        check_grounding("Turn to 270.", &turn(270, TurnDirection::Shortest)).is_ok(),
        "a turn binds its number"
    );
    assert!(
        check_grounding("Proceed to 270.", &turn(270, TurnDirection::Shortest)).is_err(),
        "no heading word and no turn"
    );
    assert!(check_grounding("Steer 200 degrees.", &turn(200, TurnDirection::Shortest)).is_ok());
    assert!(
        check_grounding("Turn to 200 degrees.", &turn(200, TurnDirection::Shortest)).is_ok(),
        "the unit after the number binds it"
    );
    let speed = Directive::Speed { speed_mps: 2.0 };
    assert!(check_grounding("Reduce speed to 2.", &speed).is_ok());
    assert!(check_grounding("Fly at 2 metres per second.", &speed).is_ok());
    assert!(check_grounding("Fly heading 2.", &speed).is_err());
}

/// The grounding check must stop no correct answer of the suites. Each
/// expected directive of each case is grounded in its own message.
#[test]
fn every_expected_directive_of_the_suites_is_grounded_in_its_message() {
    for text in [
        include_str!("../../../../tools/intent-pilot/suites/atc-open-01.json"),
        include_str!("../../../../tools/intent-pilot/suites/atc-heldout-01.json"),
    ] {
        let suite = match crate::eval::Suite::parse(text) {
            Ok(suite) => suite,
            Err(error) => panic!("the suite parses: {error}"),
        };
        for case in &suite.cases {
            if let Err(refusal) = check_grounding(&case.message, &case.expect) {
                panic!("{}: {refusal} (message: {})", case.id, case.message);
            }
        }
    }
}

#[test]
fn a_word_that_the_float_parser_accepts_is_not_a_number() {
    let not_a_number = Directive::Altitude { height_m: f64::NAN };
    assert!(check_grounding("Climb to nan metres, inf.", &not_a_number).is_err());
}

#[test]
fn a_turn_side_must_be_said_and_a_said_side_must_not_be_lost() {
    let short = turn(10, TurnDirection::Shortest);
    assert!(
        check_grounding("Come right to heading 010.", &short).is_err(),
        "the side was lost"
    );
    assert!(check_grounding("Fly heading 010.", &short).is_ok());
    assert!(
        check_grounding("Fly heading 010.", &heading(10)).is_err(),
        "the side was made up"
    );
    let right = turn(10, TurnDirection::Right);
    assert!(
        check_grounding("Turn left heading 010.", &right).is_err(),
        "the wrong side"
    );
}
