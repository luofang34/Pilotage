//! Control stimulus waveforms.

use serde::{Deserialize, Serialize};

use crate::{
    ArtifactIdentity, MAX_WAVE_COMPONENTS, ValidationError,
    validation::{duration, finite, nonempty_count, range},
};

/// One component of a multisine waveform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SineComponent {
    /// The normalized component amplitude.
    pub amplitude: f64,
    /// The component frequency in hertz.
    pub frequency_hz: f64,
    /// The component phase in radians.
    pub phase_rad: f64,
}

/// A bounded control stimulus.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Waveform {
    /// Hold one normalized value.
    Step {
        /// The normalized value.
        value: f64,
    },
    /// Move between two normalized values.
    Ramp {
        /// The first normalized value.
        from: f64,
        /// The final normalized value.
        to: f64,
        /// The ramp duration in simulator nanoseconds.
        duration_ns: u64,
    },
    /// Hold a normalized value for a fixed duration.
    Pulse {
        /// The normalized value.
        value: f64,
        /// The pulse duration in simulator nanoseconds.
        duration_ns: u64,
    },
    /// Change from one normalized value to its opposite test value.
    Reversal {
        /// The first normalized value.
        first: f64,
        /// The second normalized value.
        second: f64,
        /// The hold time for each value in simulator nanoseconds.
        dwell_ns: u64,
    },
    /// Combine multiple sine components.
    Multisine {
        /// The normalized constant value.
        bias: f64,
        /// The sine components.
        components: Vec<SineComponent>,
        /// The waveform duration in simulator nanoseconds.
        duration_ns: u64,
    },
    /// Replay an immutable recorded control artifact.
    Recorded {
        /// The recorded control artifact identity.
        source: ArtifactIdentity,
    },
}

impl Waveform {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::Step { value } => normalized(&format!("{field}.value"), *value),
            Self::Ramp {
                from,
                to,
                duration_ns,
            } => {
                normalized(&format!("{field}.from"), *from)?;
                normalized(&format!("{field}.to"), *to)?;
                duration(&format!("{field}.duration_ns"), *duration_ns)
            }
            Self::Pulse { value, duration_ns } => {
                normalized(&format!("{field}.value"), *value)?;
                duration(&format!("{field}.duration_ns"), *duration_ns)
            }
            Self::Reversal {
                first,
                second,
                dwell_ns,
            } => {
                normalized(&format!("{field}.first"), *first)?;
                normalized(&format!("{field}.second"), *second)?;
                duration(&format!("{field}.dwell_ns"), *dwell_ns)
            }
            Self::Multisine {
                bias,
                components,
                duration_ns,
            } => validate_multisine(field, *bias, components, *duration_ns),
            Self::Recorded { source } => source.validate(&format!("{field}.source")),
        }
    }
}

fn normalized(field: &str, value: f64) -> Result<(), ValidationError> {
    range(field, value, -1.0, 1.0)
}

fn validate_multisine(
    field: &str,
    bias: f64,
    components: &[SineComponent],
    duration_ns: u64,
) -> Result<(), ValidationError> {
    normalized(&format!("{field}.bias"), bias)?;
    nonempty_count(
        &format!("{field}.components"),
        components.len(),
        MAX_WAVE_COMPONENTS,
    )?;
    duration(&format!("{field}.duration_ns"), duration_ns)?;
    for (index, component) in components.iter().enumerate() {
        normalized(
            &format!("{field}.components[{index}].amplitude"),
            component.amplitude,
        )?;
        positive_frequency(field, index, component.frequency_hz)?;
        finite(
            &format!("{field}.components[{index}].phase_rad"),
            component.phase_rad,
        )?;
    }
    let envelope = components.iter().fold(bias.abs(), |total, component| {
        total + component.amplitude.abs()
    });
    range(&format!("{field}.envelope"), envelope, 0.0, 1.0)
}

fn positive_frequency(field: &str, index: usize, value: f64) -> Result<(), ValidationError> {
    range(
        &format!("{field}.components[{index}].frequency_hz"),
        value,
        f64::MIN_POSITIVE,
        f64::MAX,
    )
}
