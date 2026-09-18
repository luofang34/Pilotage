//! Scoring: each failure class stays apart from a wrong answer.

#![allow(clippy::panic)]

use std::collections::BTreeMap;

use super::{Outcome, Suite, score_case, summarize};
use crate::directive::{Arrival, Directive, TurnDirection};
use crate::model_port::ModelReply;

const SUITE: &str = r#"{
  "id": "s", "description": "d", "held_out": true,
  "envelope": {"kinds": ["direct_to", "heading", "return_to_base", "unable"],
               "fixes": ["ALPHA", "BRAVO", "HOME"], "procedures": [],
               "height_m": {"min": 2, "max": 30}, "speed_mps": {"min": 0.5, "max": 3}},
  "legend": "",
  "cases": [
    {"id": "c1", "category": "direct", "message": "proceed direct BRAVO and land",
     "expect": {"kind": "direct_to", "fix": "BRAVO", "on_arrival": "land"}},
    {"id": "c2", "category": "heading", "message": "turn left heading 270",
     "expect": {"kind": "heading", "degrees": 270, "turn": "left"}},
    {"id": "c3", "category": "chatter", "message": "nice weather today",
     "expect": {"kind": "unable", "reason": ""}}
  ]
}"#;

fn suite() -> Suite {
    match Suite::parse(SUITE) {
        Ok(suite) => suite,
        Err(error) => panic!("the test suite parses: {error}"),
    }
}

fn reply(directive: Directive, probabilities: &[(&str, f64)]) -> ModelReply {
    ModelReply {
        directive,
        probabilities: probabilities
            .iter()
            .map(|(slot, p)| ((*slot).to_owned(), *p))
            .collect::<BTreeMap<_, _>>(),
        model_ms: 100.0,
    }
}

#[test]
fn each_failure_class_has_its_own_outcome() {
    let suite = suite();
    let direct = &suite.cases[0];
    let score = |directive| score_case(direct, &suite.envelope, Ok(&reply(directive, &[]))).outcome;

    let right = Directive::DirectTo {
        fix: "BRAVO".into(),
        on_arrival: Arrival::Land,
    };
    assert_eq!(score(right), Outcome::Correct);
    let wrong_fix = Directive::DirectTo {
        fix: "ALPHA".into(),
        on_arrival: Arrival::Land,
    };
    assert_eq!(score(wrong_fix), Outcome::WrongSlot);
    assert_eq!(score(Directive::ReturnToBase {}), Outcome::WrongKind);
    let unable = Directive::Unable { reason: "?".into() };
    assert_eq!(score(unable), Outcome::Unable);
    let outside = Directive::DirectTo {
        fix: "ZULU".into(),
        on_arrival: Arrival::Land,
    };
    assert!(matches!(score(outside), Outcome::Refused { .. }));
    let fault = score_case(direct, &suite.envelope, Err("timeout".into()));
    assert!(matches!(fault.outcome, Outcome::Fault { .. }));
}

#[test]
fn chatter_is_correct_only_when_the_model_says_unable() {
    let suite = suite();
    let chatter = &suite.cases[2];
    let unable = reply(
        Directive::Unable {
            reason: "small talk".into(),
        },
        &[],
    );
    assert_eq!(
        score_case(chatter, &suite.envelope, Ok(&unable)).outcome,
        Outcome::Correct
    );
    let invented = reply(Directive::ReturnToBase {}, &[]);
    assert_eq!(
        score_case(chatter, &suite.envelope, Ok(&invented)).outcome,
        Outcome::WrongKind
    );
}

#[test]
fn the_report_shows_when_probability_does_not_separate_right_from_wrong() {
    let suite = suite();
    let right = reply(
        Directive::DirectTo {
            fix: "BRAVO".into(),
            on_arrival: Arrival::Land,
        },
        &[("kind", 0.9), ("fix", 0.62), ("on_arrival", 0.8)],
    );
    let wrong = reply(
        Directive::Heading {
            degrees: 90,
            turn: TurnDirection::Left,
        },
        &[("kind", 0.99), ("degrees", 0.97), ("turn", 0.9)],
    );
    let results = [
        score_case(&suite.cases[0], &suite.envelope, Ok(&right)),
        score_case(&suite.cases[1], &suite.envelope, Ok(&wrong)),
    ];
    let report = summarize(&results);
    assert_eq!((report.cases.correct, report.cases.total), (1, 2));
    assert_eq!(
        report.by_slot.get("degrees").map(|t| (t.correct, t.total)),
        Some((0, 1))
    );
    assert_eq!(report.by_slot.get("turn").map(|t| t.correct), Some(1));
    assert_eq!(report.min_probability_when_correct, Some(0.62));
    assert_eq!(report.max_probability_when_wrong, Some(0.97));
}

#[test]
fn a_suite_with_a_case_outside_its_envelope_is_refused() {
    let bad = SUITE.replace(
        r#""fix": "BRAVO", "on_arrival": "land"}}"#,
        r#""fix": "ZULU", "on_arrival": "land"}}"#,
    );
    assert!(Suite::parse(&bad).is_err());
    let twice = SUITE.replace(r#""id": "c2""#, r#""id": "c1""#);
    assert!(Suite::parse(&twice).is_err());
}

#[test]
fn a_wrong_answer_that_the_message_does_not_support_is_stopped_and_not_flown() {
    let suite = suite();
    // "turn left heading 270": one model misreads a digit, one invents a fix.
    let misread = reply(
        Directive::Heading {
            degrees: 220,
            turn: TurnDirection::Left,
        },
        &[],
    );
    let invented = reply(
        Directive::DirectTo {
            fix: "ALPHA".into(),
            on_arrival: Arrival::Land,
        },
        &[],
    );
    // "proceed direct BRAVO and land": a hold in place of the landing that
    // the message asks for is a lost instruction, and is stopped too.
    let wrong_arrival = reply(
        Directive::DirectTo {
            fix: "BRAVO".into(),
            on_arrival: Arrival::Hold,
        },
        &[],
    );
    let results = [
        score_case(&suite.cases[1], &suite.envelope, Ok(&misread)),
        score_case(&suite.cases[1], &suite.envelope, Ok(&invented)),
        score_case(&suite.cases[0], &suite.envelope, Ok(&wrong_arrival)),
    ];
    assert_eq!(
        results.iter().map(|r| r.would_fly).collect::<Vec<_>>(),
        [false, false, false]
    );
    let report = summarize(&results);
    assert_eq!((report.wrong_and_stopped, report.wrong_and_flown), (3, 0));
}
