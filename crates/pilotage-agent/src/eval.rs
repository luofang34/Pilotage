//! Scores for a model adapter: one rule for every model.
//!
//! A suite is a list of cases. A case has an operator message, optional
//! frames, and the directive that the author expects. The score compares the
//! reply with the expectation slot by slot. It keeps `unable`, a refused
//! reply and an adapter fault apart from a wrong answer, because they are
//! different failures with different costs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::capability::FlightEnvelope;
use crate::directive::{Arrival, Directive, DirectiveKind, HoldPoint, TurnDirection};
use crate::error::AgentError;
use crate::grounding::check_grounding;
use crate::model_port::{Frame, ModelReply, check_reply};

const DOCUMENT: &str = "suite";
/// Two number slots are equal inside this difference.
const NUMBER_TOLERANCE: f64 = 1e-6;
/// The probability key for the directive kind.
const KIND_SLOT: &str = "kind";

/// One case of a suite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Case {
    /// Stable identifier inside the suite.
    pub id: String,
    /// A group name for the report, such as `recall` or `heading`.
    pub category: String,
    /// The operator message.
    pub message: String,
    /// Images for the model.
    #[serde(default)]
    pub frames: Vec<Frame>,
    /// The directive that the author expects.
    pub expect: Directive,
}

/// A list of cases with the envelope and the legend that each request gets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suite {
    /// Stable identifier.
    pub id: String,
    /// What the suite measures.
    pub description: String,
    /// True when the author wrote the cases before any run and does not tune
    /// on them. The harness ledger shows how many times each adapter ran it.
    pub held_out: bool,
    /// What the agent can fly in these cases.
    pub envelope: FlightEnvelope,
    /// The legend for the names.
    pub legend: String,
    /// The cases.
    pub cases: Vec<Case>,
}

impl Suite {
    /// Parses and validates a suite document.
    pub fn parse(text: &str) -> Result<Self, AgentError> {
        let suite: Self = serde_json::from_str(text).map_err(|source| AgentError::Parse {
            document: DOCUMENT,
            source,
        })?;
        let mut seen = std::collections::BTreeSet::new();
        for case in &suite.cases {
            if !seen.insert(&case.id) {
                return Err(AgentError::Invalid {
                    document: DOCUMENT,
                    detail: format!("the case id {} occurs two times", case.id),
                });
            }
            let expected = ModelReply {
                directive: case.expect.clone(),
                probabilities: BTreeMap::new(),
                model_ms: 0.0,
            };
            // A case that the envelope refuses can never be correct.
            if let Err(refusal) = check_reply(&suite.envelope, &expected) {
                return Err(AgentError::Invalid {
                    document: DOCUMENT,
                    detail: format!("case {}: {refusal}", case.id),
                });
            }
        }
        Ok(suite)
    }
}

/// How one case ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// Every slot is correct.
    Correct,
    /// The directive kind is wrong.
    WrongKind,
    /// The kind is correct and one slot or more is wrong.
    WrongSlot,
    /// The model replied `unable` to an instruction.
    Unable,
    /// The reply did not pass the envelope check.
    Refused {
        /// The refusal, in words.
        detail: String,
    },
    /// The adapter gave no usable reply.
    Fault {
        /// The fault, in words.
        detail: String,
    },
}

/// One slot of one case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlotScore {
    /// The slot name. `kind` is the directive kind.
    pub slot: String,
    /// The expected value, as text.
    pub expected: String,
    /// The value in the reply, as text.
    pub got: Option<String>,
    /// True when they are equal.
    pub correct: bool,
    /// The model's probability for this slot, when it gave one.
    pub probability: Option<f64>,
}

/// The score of one case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaseResult {
    /// The case identifier.
    pub case_id: String,
    /// The case category.
    pub category: String,
    /// The expected directive kind.
    pub kind: DirectiveKind,
    /// How the case ended.
    pub outcome: Outcome,
    /// The directive in the reply, when the adapter gave one.
    pub reply: Option<Directive>,
    /// True when the reply passes the envelope check and the grounding check,
    /// so the agent would fly it.
    pub would_fly: bool,
    /// The score of each expected slot.
    pub slots: Vec<SlotScore>,
    /// The model time in milliseconds.
    pub model_ms: f64,
}

/// Correct answers of a total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Tally {
    /// Correct answers.
    pub correct: usize,
    /// All answers.
    pub total: usize,
}

/// The score of one adapter on one suite.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SuiteReport {
    /// Cases with every slot correct, of all cases.
    pub cases: Tally,
    /// The same tally for each expected directive kind.
    pub by_kind: BTreeMap<DirectiveKind, Tally>,
    /// The same tally for each category.
    pub by_category: BTreeMap<String, Tally>,
    /// Correct slots for each slot name.
    pub by_slot: BTreeMap<String, Tally>,
    /// Instructions that the model answered with `unable`.
    pub unable: usize,
    /// Replies that did not pass the envelope check.
    pub refused: usize,
    /// Cases with no usable reply.
    pub faults: usize,
    /// Wrong answers that pass every check. The agent would fly them. This is
    /// the number that protects or does not protect the vehicle.
    pub wrong_and_flown: usize,
    /// Wrong answers that the grounding check refuses.
    pub wrong_and_stopped: usize,
    /// Correct answers that the grounding check refuses. It is the cost of
    /// the strict check: the operator must say the instruction again.
    pub correct_and_stopped: usize,
    /// The smallest probability of a correct slot.
    pub min_probability_when_correct: Option<f64>,
    /// The largest probability of a wrong slot. When it is above the value
    /// before it, the probability does not separate correct from wrong.
    pub max_probability_when_wrong: Option<f64>,
    /// The median model time in milliseconds.
    pub median_model_ms: f64,
}

/// Scores one case. `reply` is the adapter's reply or its fault.
#[must_use]
pub fn score_case(
    case: &Case,
    envelope: &FlightEnvelope,
    reply: Result<&ModelReply, String>,
) -> CaseResult {
    let expected = slots_of(&case.expect);
    let result = |outcome, slots, model_ms| CaseResult {
        case_id: case.id.clone(),
        category: case.category.clone(),
        kind: case.expect.kind(),
        outcome,
        reply: None,
        would_fly: false,
        slots,
        model_ms,
    };
    let reply = match reply {
        Ok(reply) => reply,
        Err(detail) => return result(Outcome::Fault { detail }, Vec::new(), 0.0),
    };
    if let Err(refusal) = check_reply(envelope, reply) {
        let detail = refusal.to_string();
        return result(Outcome::Refused { detail }, Vec::new(), reply.model_ms);
    }
    let got = slots_of(&reply.directive);
    let slots: Vec<SlotScore> = expected
        .iter()
        .map(|(slot, want)| {
            let value = got.iter().find(|(name, _)| name == slot).map(|(_, v)| v);
            SlotScore {
                slot: (*slot).to_owned(),
                expected: want.clone(),
                got: value.cloned(),
                correct: value.is_some_and(|value| same(want, value)),
                probability: reply.probabilities.get(*slot).copied(),
            }
        })
        .collect();
    let kind_correct = reply.directive.kind() == case.expect.kind();
    let outcome = if slots.iter().all(|slot| slot.correct) {
        Outcome::Correct
    } else if reply.directive.kind() == DirectiveKind::Unable {
        Outcome::Unable
    } else if kind_correct {
        Outcome::WrongSlot
    } else {
        Outcome::WrongKind
    };
    let would_fly = reply.directive.kind() != DirectiveKind::Unable
        && check_grounding(&case.message, &reply.directive).is_ok();
    CaseResult {
        reply: Some(reply.directive.clone()),
        would_fly,
        ..result(outcome, slots, reply.model_ms)
    }
}

/// Adds the case scores into one report.
#[must_use]
pub fn summarize(results: &[CaseResult]) -> SuiteReport {
    let mut report = SuiteReport {
        cases: Tally::default(),
        by_kind: BTreeMap::new(),
        by_category: BTreeMap::new(),
        by_slot: BTreeMap::new(),
        unable: 0,
        refused: 0,
        faults: 0,
        wrong_and_flown: 0,
        wrong_and_stopped: 0,
        correct_and_stopped: 0,
        min_probability_when_correct: None,
        max_probability_when_wrong: None,
        median_model_ms: 0.0,
    };
    let mut times: Vec<f64> = Vec::new();
    for result in results {
        let correct = result.outcome == Outcome::Correct;
        add(&mut report.cases, correct);
        add(report.by_kind.entry(result.kind).or_default(), correct);
        add(
            report
                .by_category
                .entry(result.category.clone())
                .or_default(),
            correct,
        );
        let flies =
            matches!(result.reply, Some(ref reply) if reply.kind() != DirectiveKind::Unable);
        match (&result.outcome, result.would_fly) {
            (Outcome::WrongKind | Outcome::WrongSlot, true) => {
                report.wrong_and_flown = report.wrong_and_flown.wrapping_add(1);
            }
            (Outcome::WrongKind | Outcome::WrongSlot, false) => {
                report.wrong_and_stopped = report.wrong_and_stopped.wrapping_add(1);
            }
            (Outcome::Correct, false) if flies => {
                report.correct_and_stopped = report.correct_and_stopped.wrapping_add(1);
            }
            _ => {}
        }
        match result.outcome {
            Outcome::Unable => report.unable = report.unable.wrapping_add(1),
            Outcome::Refused { .. } => report.refused = report.refused.wrapping_add(1),
            Outcome::Fault { .. } => report.faults = report.faults.wrapping_add(1),
            _ => times.push(result.model_ms),
        }
        for slot in &result.slots {
            add(
                report.by_slot.entry(slot.slot.clone()).or_default(),
                slot.correct,
            );
            let Some(probability) = slot.probability else {
                continue;
            };
            let bound = if slot.correct {
                &mut report.min_probability_when_correct
            } else {
                &mut report.max_probability_when_wrong
            };
            *bound = Some(match (*bound, slot.correct) {
                (None, _) => probability,
                (Some(now), true) => now.min(probability),
                (Some(now), false) => now.max(probability),
            });
        }
    }
    times.sort_by(f64::total_cmp);
    report.median_model_ms = times.get(times.len() / 2).copied().unwrap_or(0.0);
    report
}

fn add(tally: &mut Tally, correct: bool) {
    tally.total = tally.total.wrapping_add(1);
    if correct {
        tally.correct = tally.correct.wrapping_add(1);
    }
}

/// True when two slot texts are equal, with a tolerance for numbers.
fn same(expected: &str, got: &str) -> bool {
    match (expected.parse::<f64>(), got.parse::<f64>()) {
        (Ok(expected), Ok(got)) => (expected - got).abs() <= NUMBER_TOLERANCE,
        _ => expected == got,
    }
}

/// The slots of a directive as text, the kind first. The reason of `unable`
/// is free text and is not a slot.
fn slots_of(directive: &Directive) -> Vec<(&'static str, String)> {
    let kind = serde_json::to_value(directive.kind())
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut slots = vec![(KIND_SLOT, kind)];
    match directive {
        Directive::DirectTo { fix, on_arrival } => {
            slots.push(("fix", fix.clone()));
            slots.push(("on_arrival", arrival_text(*on_arrival).to_owned()));
        }
        Directive::Heading { degrees, turn } => {
            slots.push(("degrees", degrees.to_string()));
            slots.push(("turn", turn_text(*turn).to_owned()));
        }
        Directive::Altitude { height_m } => slots.push(("height_m", height_m.to_string())),
        Directive::Speed { speed_mps } => slots.push(("speed_mps", speed_mps.to_string())),
        Directive::Hold { point } => slots.push((
            "point",
            match point {
                HoldPoint::PresentPosition => "present_position".to_owned(),
                HoldPoint::Fix(fix) => fix.clone(),
            },
        )),
        Directive::JoinProcedure { procedure } => slots.push(("procedure", procedure.clone())),
        _ => {}
    }
    slots
}

const fn arrival_text(arrival: Arrival) -> &'static str {
    match arrival {
        Arrival::Land => "land",
        Arrival::Hold => "hold",
    }
}

const fn turn_text(turn: TurnDirection) -> &'static str {
    match turn {
        TurnDirection::Left => "left",
        TurnDirection::Right => "right",
        TurnDirection::Shortest => "shortest",
    }
}

#[cfg(test)]
mod tests;
