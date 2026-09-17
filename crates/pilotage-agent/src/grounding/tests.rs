//! Grounding: what the operator said, and what a model made up.

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
fn a_directive_with_no_name_and_no_number_always_passes() {
    for directive in [
        Directive::Takeoff {},
        Directive::Land {},
        Directive::GoAround {},
        Directive::ReturnToBase {},
        Directive::Hold {
            point: HoldPoint::PresentPosition,
        },
    ] {
        assert!(check_grounding("anything", &directive).is_ok());
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
