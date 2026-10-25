//! The rules that [`super::AT_MOST`] and other rule operations name: pure
//! functions of their parameters, the same on every host.

/// The number `text` is, when it is a finite one.
///
/// # Errors
/// When `text` is not a finite number.
pub fn limit(text: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|l| l.is_finite())
        .ok_or_else(|| format!("the limit {text:?} is not a number"))
}

/// Whether `value`, a number with an optional unit after it such as
/// `21.5 C` or `21.5C`, is at most `limit`; with the verdict as a person
/// reads it.
///
/// # Errors
/// When `value` does not start with a number or `limit` is not one.
pub fn at_most(value: &str, limit_text: &str) -> Result<(bool, String), String> {
    let limit_number = limit(limit_text)?;
    let number = leading_number(value)
        .ok_or_else(|| format!("the value {value:?} does not start with a number"))?;
    let within = number <= limit_number;
    let limit_text = limit_text.trim();
    let verdict = if within {
        format!("{number} is within the limit of {limit_text}")
    } else {
        format!("{number} is above the limit of {limit_text}")
    };
    Ok((within, verdict))
}

/// The number the longest number-shaped prefix of `value` is, when it is
/// finite.
fn leading_number(value: &str) -> Option<f64> {
    let value = value.trim_start();
    let ends: Vec<usize> = value
        .char_indices()
        .map(|(at, c)| at + c.len_utf8())
        .collect();
    ends.into_iter()
        .rev()
        .find_map(|end| value[..end].parse::<f64>().ok())
        .filter(|n| n.is_finite())
}
