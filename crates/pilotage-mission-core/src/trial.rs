//! Trial action value types.

use serde::{Deserialize, Serialize};

use crate::{ArtifactIdentity, ValidationError, validation::range};

const MAX_WAVE_COMPONENTS: usize = 64;

/// A test start state relative to the first observation after reset.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartState {
    /// The north-east-down position offset in meters.
    pub relative_position_ned_m: [f64; 3],
    /// The target heading.
    pub heading: StartHeading,
}

/// The heading reference for a test start state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StartHeading {
    /// Add an offset to the first heading after reset.
    ResetOffset {
        /// The clockwise heading offset in radians.
        radians: f64,
    },
    /// Use a true heading clockwise from north.
    True {
        /// The true heading in radians.
        radians: f64,
    },
}

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
    /// Change from one value to its opposite test value.
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

impl StartState {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        for (index, value) in self.relative_position_ned_m.iter().enumerate() {
            range(
                &format!("{field}.action.target.relative_position_ned_m[{index}]"),
                *value,
                -1_000.0,
                1_000.0,
            )?;
        }
        self.heading.validate(field)
    }
}

impl StartHeading {
    fn validate(self, field: &str) -> Result<(), ValidationError> {
        let radians = match self {
            Self::ResetOffset { radians } | Self::True { radians } => radians,
        };
        range(
            &format!("{field}.action.target.heading.radians"),
            radians,
            -core::f64::consts::PI,
            core::f64::consts::PI,
        )
    }
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
            } => validate_reversal(field, *first, *second, *dwell_ns),
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

fn duration(field: &str, value: u64) -> Result<(), ValidationError> {
    if value == 0 {
        return Err(ValidationError::ZeroDuration {
            field: field.to_owned(),
        });
    }
    Ok(())
}

fn validate_reversal(
    field: &str,
    first: f64,
    second: f64,
    dwell_ns: u64,
) -> Result<(), ValidationError> {
    normalized(&format!("{field}.first"), first)?;
    normalized(&format!("{field}.second"), second)?;
    duration(&format!("{field}.dwell_ns"), dwell_ns)
}

fn validate_multisine(
    field: &str,
    bias: f64,
    components: &[SineComponent],
    duration_ns: u64,
) -> Result<(), ValidationError> {
    normalized(&format!("{field}.bias"), bias)?;
    crate::validation::nonempty_count(
        &format!("{field}.components"),
        components.len(),
        MAX_WAVE_COMPONENTS,
    )?;
    duration(&format!("{field}.duration_ns"), duration_ns)?;
    for (index, component) in components.iter().enumerate() {
        validate_sine_component(field, index, component)?;
    }
    let envelope = components.iter().fold(bias.abs(), |total, component| {
        total + component.amplitude.abs()
    });
    range(&format!("{field}.envelope"), envelope, 0.0, 1.0)
}

fn validate_sine_component(
    field: &str,
    index: usize,
    component: &SineComponent,
) -> Result<(), ValidationError> {
    normalized(
        &format!("{field}.components[{index}].amplitude"),
        component.amplitude,
    )?;
    range(
        &format!("{field}.components[{index}].frequency_hz"),
        component.frequency_hz,
        f64::MIN_POSITIVE,
        f64::MAX,
    )?;
    crate::validation::finite(
        &format!("{field}.components[{index}].phase_rad"),
        component.phase_rad,
    )
}
