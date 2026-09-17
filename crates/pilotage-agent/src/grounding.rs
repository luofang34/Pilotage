//! The grounding check: a slot value must come from the operator message.
//!
//! The envelope check refuses a name that is not permitted. It cannot refuse a
//! permitted name that the operator did not say. A model that reads "proceed
//! direct ZULU" and replies `ALPHA` passes the envelope, and the vehicle would
//! fly to `ALPHA`. This check reads the message with no model: each name and
//! each number in the directive must be in the words of the message. A wrong
//! number then becomes a refusal and not a flight.
//!
//! The check is strict on purpose. When it refuses a correct reading, the
//! operator says the instruction again with the name or the number in it.

use crate::directive::{Directive, HoldPoint, TurnDirection};
use crate::model_port::Refusal;
use crate::scenario::HOME;

/// Two numbers are equal inside this difference.
const NUMBER_TOLERANCE: f64 = 1e-6;
/// Words that name the launch point.
const HOME_WORDS: [&str; 2] = ["home", "base"];

/// One word of the message.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Number(f64),
}

/// Checks that each name and number of `directive` is in `message`.
pub fn check_grounding(message: &str, directive: &Directive) -> Result<(), Refusal> {
    let tokens = tokens(message);
    let refuse = |slot, value: String| Err(Refusal::NotInMessage { slot, value });
    match directive {
        Directive::DirectTo { fix, .. }
        | Directive::Hold {
            point: HoldPoint::Fix(fix),
        } if !names(&tokens, fix) => refuse("fix", fix.clone()),
        Directive::JoinProcedure { procedure } if !names(&tokens, procedure) => {
            refuse("procedure", procedure.clone())
        }
        Directive::Heading { degrees, .. } if !has_heading(&tokens, *degrees) => {
            refuse("degrees", degrees.to_string())
        }
        Directive::Heading { turn, .. } if !turn_is_said(&tokens, *turn) => {
            refuse("turn", format!("{turn:?}"))
        }
        Directive::Altitude { height_m } if !has_number(&tokens, *height_m) => {
            refuse("height_m", height_m.to_string())
        }
        Directive::Speed { speed_mps } if !has_number(&tokens, *speed_mps) => {
            refuse("speed_mps", speed_mps.to_string())
        }
        _ => Ok(()),
    }
}

/// True when the message says `name`. A name such as `RNAV27` is a letter part
/// and a number part, and the message can say them apart: "RNAV two seven",
/// "the RNAV approach runway 27".
fn names(tokens: &[Token], name: &str) -> bool {
    if name == HOME {
        return HOME_WORDS
            .iter()
            .any(|word| tokens.contains(&Token::Word((*word).to_owned())));
    }
    split_runs(&name.to_lowercase())
        .iter()
        .all(|part| match part {
            Token::Word(_) => tokens.contains(part),
            Token::Number(number) => has_number(tokens, *number),
        })
}

/// A side of a turn must be said, and a side that was said must not be lost.
/// The short way is correct only when the message names no side.
fn turn_is_said(tokens: &[Token], turn: TurnDirection) -> bool {
    let says = |word: &str| tokens.contains(&Token::Word(word.to_owned()));
    match turn {
        TurnDirection::Left => says("left") && !says("right"),
        TurnDirection::Right => says("right") && !says("left"),
        TurnDirection::Shortest => !says("left") && !says("right"),
    }
}

fn has_number(tokens: &[Token], wanted: f64) -> bool {
    tokens.iter().any(|token| {
        matches!(token, Token::Number(number) if (number - wanted).abs() <= NUMBER_TOLERANCE)
    })
}

/// Heading 360 is heading 0, so the numbers are compared after the wrap.
fn has_heading(tokens: &[Token], degrees: u16) -> bool {
    tokens.iter().any(|token| {
        matches!(token, Token::Number(number)
            if number.fract() == 0.0 && (number.rem_euclid(360.0) - f64::from(degrees)).abs() <= NUMBER_TOLERANCE)
    })
}

/// The message as words and numbers. Spoken digits become one number: "two
/// seven zero" is 270, and "niner" is 9.
fn tokens(message: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut spoken = String::new();
    for raw in message.to_lowercase().split_whitespace() {
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric());
        if let Some(digit) = spoken_digit(word) {
            spoken.push(digit);
            continue;
        }
        flush(&mut spoken, &mut tokens);
        // Only a word that starts with a digit is a number. The float parser
        // also accepts words such as `nan` and `inf`.
        let numeric = word.starts_with(|c: char| c.is_ascii_digit());
        match word.parse::<f64>() {
            Ok(number) if numeric && number.is_finite() => tokens.push(Token::Number(number)),
            _ => tokens.extend(split_runs(word)),
        }
    }
    flush(&mut spoken, &mut tokens);
    tokens
}

fn flush(spoken: &mut String, tokens: &mut Vec<Token>) {
    if let Ok(number) = spoken.parse::<f64>() {
        tokens.push(Token::Number(number));
    }
    spoken.clear();
}

fn spoken_digit(word: &str) -> Option<char> {
    Some(match word {
        "zero" => '0',
        "one" => '1',
        "two" => '2',
        "three" => '3',
        "four" => '4',
        "five" => '5',
        "six" => '6',
        "seven" => '7',
        "eight" => '8',
        "nine" | "niner" => '9',
        _ => return None,
    })
}

/// Splits a word into its letter runs and its digit runs: `rnav27` is `rnav`
/// and 27.
fn split_runs(word: &str) -> Vec<Token> {
    let mut runs = Vec::new();
    let mut run = String::new();
    let mut digits = false;
    for character in word.chars().filter(|c| c.is_alphanumeric()) {
        if !run.is_empty() && character.is_ascii_digit() != digits {
            push_run(&mut runs, &run, digits);
            run.clear();
        }
        digits = character.is_ascii_digit();
        run.push(character);
    }
    push_run(&mut runs, &run, digits);
    runs
}

fn push_run(runs: &mut Vec<Token>, run: &str, digits: bool) {
    if run.is_empty() {
        return;
    }
    match run.parse::<f64>() {
        Ok(number) if digits => runs.push(Token::Number(number)),
        _ => runs.push(Token::Word(run.to_owned())),
    }
}

#[cfg(test)]
mod tests;
