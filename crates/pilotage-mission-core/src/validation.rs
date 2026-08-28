//! Primitive mission document validation.

use std::collections::HashSet;
use std::hash::Hash;

use crate::{Digest, MAX_TEXT_BYTES, MISSION_SCHEMA_VERSION, ValidationError};

pub(crate) fn schema(actual: u16) -> Result<(), ValidationError> {
    if actual == MISSION_SCHEMA_VERSION {
        return Ok(());
    }
    Err(ValidationError::UnsupportedSchemaVersion {
        actual,
        expected: MISSION_SCHEMA_VERSION,
    })
}

pub(crate) fn text(field: &str, value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        return Err(ValidationError::EmptyText {
            field: field.to_owned(),
        });
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(ValidationError::TextTooLong {
            field: field.to_owned(),
            size: value.len(),
            limit: MAX_TEXT_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn digest(field: &str, value: Digest) -> Result<(), ValidationError> {
    if value.is_zero() {
        return Err(ValidationError::ZeroDigest {
            field: field.to_owned(),
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
    count_with_limit(field, count, limit)
}

pub(crate) fn count_with_limit(
    field: &str,
    count: usize,
    limit: usize,
) -> Result<(), ValidationError> {
    if count > limit {
        return Err(ValidationError::TooManyItems {
            field: field.to_owned(),
            count,
            limit,
        });
    }
    Ok(())
}

pub(crate) fn duplicate<T: Eq + Hash + Copy>(values: &[T]) -> Option<T> {
    let mut seen = HashSet::with_capacity(values.len());
    values.iter().copied().find(|value| !seen.insert(*value))
}

pub(crate) fn finite(field: &str, value: f64) -> Result<(), ValidationError> {
    if !value.is_finite() {
        return Err(ValidationError::NonFinite {
            field: field.to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn nonzero_u64(field: &str, value: u64) -> Result<(), ValidationError> {
    if value == 0 {
        return Err(ValidationError::ZeroDuration {
            field: field.to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn range(
    field: &str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), ValidationError> {
    finite(field, value)?;
    if value < minimum || value > maximum {
        return Err(ValidationError::OutOfRange {
            field: field.to_owned(),
            actual: value,
            minimum,
            maximum,
        });
    }
    Ok(())
}
