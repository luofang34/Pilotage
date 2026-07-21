//! Typed errors for device-profile parsing and loading (ADR-0007).

/// Errors that can occur while parsing or loading a device profile.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ProfileError {
    /// The profile bytes were not valid UTF-8.
    #[error("device profile is not valid UTF-8: {source}")]
    InvalidUtf8 {
        /// Underlying UTF-8 decode failure.
        #[source]
        source: Utf8ErrorEq,
    },

    /// The profile JSON could not be deserialized into the schema.
    #[error("device profile JSON is malformed: {message}")]
    MalformedJson {
        /// Description of the deserialization failure.
        message: String,
    },

    /// The profile declared a `schema_version` this crate does not support.
    #[error("device profile schema_version {found} is not supported (expected {expected})")]
    UnsupportedSchemaVersion {
        /// The `schema_version` found in the profile.
        found: u32,
        /// The `schema_version` this crate supports.
        expected: u32,
    },

    /// An axis entry named a logical axis outside the well-known table.
    #[error("device profile references unknown logical axis name {name:?}")]
    UnknownAxisName {
        /// The unrecognized logical axis name.
        name: String,
    },

    /// A button entry named a logical button outside the well-known table.
    #[error("device profile references unknown logical button name {name:?}")]
    UnknownButtonName {
        /// The unrecognized logical button name.
        name: String,
    },

    /// A calibration range was degenerate or out of order in a way that
    /// would make normalization ill-defined or sign-inverting (e.g.
    /// `min == max`, or `center` not strictly between `min` and `max`).
    #[error(
        "device profile axis {source_index} has a degenerate calibration range: min={min}, center={center}, max={max}"
    )]
    DegenerateCalibration {
        /// Source index of the offending axis.
        source_index: usize,
        /// The calibration `min` value.
        min: f32,
        /// The calibration `center` value.
        center: f32,
        /// The calibration `max` value.
        max: f32,
    },

    /// Two axis entries in one profile claim the same `source_index`, or a
    /// button `source_index` is repeated. Within a single profile that is
    /// ambiguous (the cross-layer precedence rule resolves collisions
    /// *between* layers, never inside one), so it is rejected at load time.
    #[error("device profile repeats {kind} source_index {source_index}")]
    DuplicateSourceIndex {
        /// Whether the collision is between axis or button entries.
        kind: EntryKind,
        /// The repeated source index.
        source_index: usize,
    },

    /// Two entries of the same kind map to one logical name, which would
    /// make two physical inputs race for a single logical axis or button.
    #[error("device profile maps {kind} logical name {name:?} more than once")]
    DuplicateLogicalName {
        /// Whether the collision is between axis or button entries.
        kind: EntryKind,
        /// The repeated logical name.
        name: String,
    },

    /// An axis `deadzone` was outside `[0.0, 1.0)`, the range over which the
    /// normalization pipeline stays monotonic (a deadzone `>= 1.0` would clamp
    /// every input to zero; a negative one is meaningless).
    #[error("device profile axis {source_index} deadzone {value} is outside [0.0, 1.0)")]
    DeadzoneOutOfRange {
        /// Source index of the offending axis.
        source_index: usize,
        /// The out-of-range deadzone value.
        value: f32,
    },

    /// An axis `expo` was outside `[-0.99, 10.0]`, the range over which the
    /// response curve stays bounded and single-valued.
    #[error("device profile axis {source_index} expo {value} is outside [-0.99, 10.0]")]
    ExpoOutOfRange {
        /// Source index of the offending axis.
        source_index: usize,
        /// The out-of-range expo value.
        value: f32,
    },

    /// An axis response-curve field (`deadzone` or `expo`) was `NaN` or
    /// infinite, so it could never define a usable curve.
    #[error("device profile axis {source_index} has a non-finite {field}")]
    NonFiniteAxisValue {
        /// Source index of the offending axis.
        source_index: usize,
        /// Which response-curve field was non-finite.
        field: &'static str,
    },
}

/// Distinguishes axis entries from button entries in duplicate-entry errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// The duplicate is among `axes` entries.
    Axis,
    /// The duplicate is among `buttons` entries.
    Button,
}

impl core::fmt::Display for EntryKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Axis => f.write_str("axis"),
            Self::Button => f.write_str("button"),
        }
    }
}

/// A `PartialEq`/`Eq`-friendly wrapper around [`std::str::Utf8Error`] so
/// [`ProfileError`] can derive equality for tests without losing the
/// original error via `#[source]`.
#[derive(Debug)]
pub struct Utf8ErrorEq(pub core::str::Utf8Error);

impl core::fmt::Display for Utf8ErrorEq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for Utf8ErrorEq {}

impl PartialEq for Utf8ErrorEq {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Utf8ErrorEq {}
