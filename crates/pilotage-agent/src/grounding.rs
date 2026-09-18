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

use crate::directive::{Arrival, Directive, HoldPoint, TurnDirection};
use crate::model_port::Refusal;
use crate::scenario::HOME;

/// Two numbers are equal inside this difference.
const NUMBER_TOLERANCE: f64 = 1e-6;
/// Words that name the launch point.
const HOME_WORDS: [&str; 2] = ["home", "base"];
/// Words that ask for a landing.
const LAND_WORDS: [&str; 5] = ["land", "landing", "ground", "down", "touchdown"];
/// Words that ask for a takeoff.
const TAKEOFF_WORDS: [&str; 8] = [
    "takeoff",
    "take",
    "off",
    "airborne",
    "lift",
    "depart",
    "departure",
    "launch",
];
/// Words that ask for a return to the launch point.
const RETURN_WORDS: [&str; 6] = ["return", "home", "base", "rtb", "back", "launch"];
/// Words that ask for a go-around.
const GO_AROUND_WORDS: [&str; 1] = ["around"];
/// Words that ask for a hold.
const HOLD_WORDS: [&str; 7] = [
    "hold", "holding", "hover", "orbit", "stop", "stay", "remain",
];
/// Words that bind the number after them to a heading. A turn has a side or
/// a heading and never a height, so the turn words bind too.
const HEADING_WORDS: [&str; 6] = ["heading", "hdg", "steer", "turn", "left", "right"];
/// Words that bind the number before them to a heading.
const DEGREE_WORDS: [&str; 2] = ["degrees", "deg"];
/// Words that bind the number after them to a speed.
const SPEED_WORDS: [&str; 3] = ["speed", "knots", "kts"];
/// How many tokens before a number a binding word can stand.
const BINDING_REACH: usize = 2;

/// One word of the message.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Number(f64),
}

/// The slot that the words before a number bind it to. A number after
/// "heading" is not a height, whatever its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binding {
    Heading,
    Speed,
    Free,
}

/// Checks that each name and number of `directive` is in `message`.
pub fn check_grounding(message: &str, directive: &Directive) -> Result<(), Refusal> {
    let tokens = tokens(message);
    let refuse = |slot, value: String| Err(Refusal::NotInMessage { slot, value });
    let says_any = |words: &[&str]| words.iter().any(|word| says(&tokens, word));
    match directive {
        Directive::DirectTo { fix, .. }
        | Directive::Hold {
            point: HoldPoint::Fix(fix),
        } if !names(&tokens, fix) => refuse("fix", fix.clone()),
        // The arrival is an instruction of its own: a landing that the
        // operator did not ask for is the one that matters most.
        Directive::DirectTo { fix, on_arrival } if !arrival_is_said(&tokens, fix, *on_arrival) => {
            refuse("on_arrival", format!("{on_arrival:?}"))
        }
        Directive::JoinProcedure { procedure } if !names(&tokens, procedure) => {
            refuse("procedure", procedure.clone())
        }
        Directive::Heading { degrees, .. } if !has_heading(&tokens, *degrees) => {
            refuse("degrees", degrees.to_string())
        }
        Directive::Heading { turn, .. } if !turn_is_said(&tokens, *turn) => {
            refuse("turn", format!("{turn:?}"))
        }
        Directive::Altitude { height_m } if !has_number(&tokens, *height_m, Binding::Free) => {
            refuse("height_m", height_m.to_string())
        }
        Directive::Speed { speed_mps } if !has_number(&tokens, *speed_mps, Binding::Speed) => {
            refuse("speed_mps", speed_mps.to_string())
        }
        Directive::Land {} if !says_any(&LAND_WORDS) => refuse("kind", "land".to_owned()),
        Directive::Takeoff {} if !says_any(&TAKEOFF_WORDS) => refuse("kind", "takeoff".to_owned()),
        Directive::ReturnToBase {} if !says_any(&RETURN_WORDS) => {
            refuse("kind", "return_to_base".to_owned())
        }
        Directive::GoAround {} if !says_any(&GO_AROUND_WORDS) => {
            refuse("kind", "go_around".to_owned())
        }
        Directive::Hold {
            point: HoldPoint::PresentPosition,
        } if !says_any(&HOLD_WORDS) => refuse("kind", "hold".to_owned()),
        _ => Ok(()),
    }
}

fn says(tokens: &[Token], word: &str) -> bool {
    tokens.contains(&Token::Word(word.to_owned()))
}

/// A landing must be asked for. A return to the launch point asks for one.
/// A hold is the arrival when no landing is asked for, so a hold is refused
/// only when the message asks to land.
fn arrival_is_said(tokens: &[Token], fix: &str, arrival: Arrival) -> bool {
    let lands = LAND_WORDS.iter().any(|word| says(tokens, word));
    let returns = fix == HOME && RETURN_WORDS.iter().any(|word| says(tokens, word));
    match arrival {
        Arrival::Land => lands || returns,
        Arrival::Hold => !lands,
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
            Token::Number(number) => has_number(tokens, *number, Binding::Free),
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

/// True when the message says `wanted` as a number that is not bound to a
/// different slot. A number bound to a heading is not a height or a speed.
/// A free number can be a speed, because a speed is often said with its unit
/// only ("2 metres per second").
fn has_number(tokens: &[Token], wanted: f64, slot: Binding) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        matches!(token, Token::Number(number)
            if (number - wanted).abs() <= NUMBER_TOLERANCE
                && matches!((binding(tokens, index), slot), (Binding::Free, _) | (Binding::Speed, Binding::Speed)))
    })
}

/// Heading 360 is heading 0, so the numbers are compared after the wrap. The
/// number must follow a heading word: "climb to 12 metres, turn left heading
/// 270" has one heading in it.
fn has_heading(tokens: &[Token], degrees: u16) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        matches!(token, Token::Number(number)
            if number.fract() == 0.0
                && (number.rem_euclid(360.0) - f64::from(degrees)).abs() <= NUMBER_TOLERANCE
                && binding(tokens, index) == Binding::Heading)
    })
}

/// The slot that the words around the token at `index` bind it to: a
/// heading word before it, a degree word after it, or a speed word before it.
fn binding(tokens: &[Token], index: usize) -> Binding {
    if let Some(Token::Word(after)) = tokens.get(index.wrapping_add(1))
        && DEGREE_WORDS.contains(&after.as_str())
    {
        return Binding::Heading;
    }
    let start = index.saturating_sub(BINDING_REACH);
    for token in tokens[start..index].iter().rev() {
        let Token::Word(word) = token else {
            continue;
        };
        if HEADING_WORDS.contains(&word.as_str()) {
            return Binding::Heading;
        }
        if SPEED_WORDS.contains(&word.as_str()) {
            return Binding::Speed;
        }
    }
    Binding::Free
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
