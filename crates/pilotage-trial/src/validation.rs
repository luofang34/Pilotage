//! Shared validation rules for trial data.

use crate::{Digest, ValidationError};

pub(crate) fn schema(
    document: &'static str,
    actual: u16,
    expected: u16,
) -> Result<(), ValidationError> {
    if actual == expected {
        return Ok(());
    }
    Err(ValidationError::UnsupportedSchemaVersion {
        document,
        actual,
        expected,
    })
}

pub(crate) fn text(field: &str, value: &str, limit: usize) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        return Err(ValidationError::EmptyText {
            field: field.to_owned(),
        });
    }
    if value.len() > limit {
        return Err(ValidationError::TextTooLong {
            field: field.to_owned(),
            size: value.len(),
            limit,
        });
    }
    Ok(())
}

pub(crate) fn optional_text(
    field: &str,
    value: Option<&str>,
    limit: usize,
) -> Result<(), ValidationError> {
    match value {
        Some(value) => text(field, value, limit),
        None => Ok(()),
    }
}

pub(crate) fn digest(field: &str, value: Digest) -> Result<(), ValidationError> {
    if value.is_zero() {
        return Err(ValidationError::ZeroDigest {
            field: field.to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn count(field: &str, count: usize, limit: usize) -> Result<(), ValidationError> {
    if count > limit {
        return Err(ValidationError::TooManyItems {
            field: field.to_owned(),
            count,
            limit,
        });
    }
    Ok(())
}

pub(crate) fn nonempty_count(
    field: &str,
    count: usize,
    limit: usize,
) -> Result<(), ValidationError> {
    if count == 0 {
        return Err(ValidationError::EmptyList {
            field: field.to_owned(),
        });
    }
    self::count(field, count, limit)
}

pub(crate) fn unique<T: PartialEq>(field: &str, values: &[T]) -> Result<(), ValidationError> {
    for (index, value) in values.iter().enumerate() {
        if values[..index].contains(value) {
            return Err(ValidationError::DuplicateItem {
                field: field.to_owned(),
                index,
            });
        }
    }
    Ok(())
}

pub(crate) fn finite(field: &str, value: f64) -> Result<(), ValidationError> {
    if value.is_finite() {
        return Ok(());
    }
    Err(ValidationError::NonFinite {
        field: field.to_owned(),
    })
}

pub(crate) fn range(
    field: &str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), ValidationError> {
    finite(field, value)?;
    if (minimum..=maximum).contains(&value) {
        return Ok(());
    }
    Err(ValidationError::OutOfRange {
        field: field.to_owned(),
        actual: value,
        minimum,
        maximum,
    })
}

pub(crate) fn duration(field: &str, value: u64) -> Result<(), ValidationError> {
    if value > 0 {
        return Ok(());
    }
    Err(ValidationError::ZeroDuration {
        field: field.to_owned(),
    })
}
