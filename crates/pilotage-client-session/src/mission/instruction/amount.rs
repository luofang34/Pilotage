//! The amounts an instruction's words state, and whether a move's fraction is one of
//! them. The interpreter never supplies a number: a fraction stands only when the words
//! state that value for that member, and when the words do not tie one amount to the
//! member, the person says it again.

use super::InstructionError;

/// How far apart two fractions may be and still be the same amount, so that a third
/// matches the interpreter's 0.333.
const SAME: f64 = 0.002;

/// One amount the words state, and the word it stands at.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stated {
    at: usize,
    value: f64,
}

/// Checks that `fraction`, the share of `from`'s lines a move takes, is an amount the
/// `words` state for `from`. With one amount stated, it must be that amount; with more
/// than one, it must be the amount nearest the words that name `from`.
///
/// # Errors
///
/// [`InstructionError::NeedsAmount`] when the words state no amount, and
/// [`InstructionError::UnsureAmount`] when they do not tie `fraction` to `from`.
pub(super) fn ground(words: &str, from: &str, fraction: f64) -> Result<(), InstructionError> {
    let tokens = tokens(words);
    let stated = stated(&tokens);
    let unsure = || InstructionError::UnsureAmount {
        from: from.to_owned(),
        fraction,
    };
    let first = stated.first().ok_or(InstructionError::NeedsAmount)?;
    if stated.iter().all(|s| same(s.value, first.value)) {
        return if same(first.value, fraction) {
            Ok(())
        } else {
            Err(unsure())
        };
    }
    let named = mentions(&tokens, from);
    let [at] = named.as_slice() else {
        return Err(unsure());
    };
    let distance = |s: &Stated| s.at.abs_diff(*at);
    let nearest = stated.iter().map(distance).min().unwrap_or(usize::MAX);
    let mut closest = stated.iter().filter(|s| distance(s) == nearest);
    match (closest.next(), closest.next()) {
        (Some(only), None) if same(only.value, fraction) => Ok(()),
        _ => Err(unsure()),
    }
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= SAME
}

/// The words, lower case, split at spaces and at marks other than those inside names
/// and numbers: `SIM-B1`, `0.5`, `25%`, `1/3`.
fn tokens(words: &str) -> Vec<String> {
    words
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || "-_.%/".contains(c)))
        .map(|t| t.trim_matches(|c: char| ".-_/".contains(c)).to_owned())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Where the words name `name`: the first word of each place its words follow in order.
fn mentions(tokens: &[String], name: &str) -> Vec<usize> {
    let name = self::tokens(name);
    if name.is_empty() {
        return Vec::new();
    }
    tokens
        .windows(name.len())
        .enumerate()
        .filter(|(_, window)| *window == name.as_slice())
        .map(|(at, _)| at)
        .collect()
}

/// Every amount the words state, in order.
fn stated(tokens: &[String]) -> Vec<Stated> {
    tokens
        .iter()
        .enumerate()
        .filter_map(|(at, token)| {
            let before = at.checked_sub(1).and_then(|b| tokens.get(b));
            let after = tokens.get(at.wrapping_add(1));
            amount(token, before.map(String::as_str), after.map(String::as_str))
                .map(|value| Stated { at, value })
        })
        .collect()
}

/// The amount one word states, read with the words next to it.
fn amount(token: &str, before: Option<&str>, after: Option<&str>) -> Option<f64> {
    let count = |whole: f64, parts: f64| match before {
        Some("two") => Some(2.0 / parts),
        Some("three") if parts > 3.0 => Some(3.0 / parts),
        _ => Some(whole / parts),
    };
    match token {
        "all" | "every" | "whole" | "entire" | "rest" | "remaining" => return Some(1.0),
        "half" => return Some(0.5),
        "third" | "thirds" => return count(1.0, 3.0),
        "quarter" | "quarters" => return count(1.0, 4.0),
        _ => {}
    }
    if let Some(number) = token.strip_suffix('%') {
        return percent(number);
    }
    if matches!(after, Some("%" | "percent")) {
        return percent(token);
    }
    if let Some((top, bottom)) = token.split_once('/') {
        let (top, bottom) = (top.parse::<f64>().ok()?, bottom.parse::<f64>().ok()?);
        return share(top / bottom);
    }
    // A bare whole number counts lines, not a share of them.
    if token.contains('.') {
        return share(token.parse().ok()?);
    }
    None
}

fn percent(number: &str) -> Option<f64> {
    share(number.parse::<f64>().ok()? / 100.0)
}

fn share(value: f64) -> Option<f64> {
    (value > 0.0 && value <= 1.0).then_some(value)
}
