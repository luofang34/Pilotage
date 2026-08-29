//! The normalized stimulus value that one waveform asks for.
//!
//! Every waveform is a pure function of the elapsed phase time, so the
//! same run seed and the same frames give the same commanded values. A
//! waveform states when it is finished; the runtime never guesses a
//! duration and never extrapolates past one.

use flight_tune::{SineComponent, Waveform};

use crate::runtime::AviateRuntimeError;
use crate::runtime::math::{clamp_normalized, require_finite};

/// What one waveform asks for at a point in its window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaveformSample {
    /// The waveform commands this bounded normalized value.
    Active(f64),
    /// The waveform reached the end of its own declared window.
    Complete,
}

/// The normalized value that one waveform commands after an elapsed time.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when a value is not finite, or when the
/// waveform replays a recorded artifact that the runtime cannot resolve.
pub fn sample(waveform: &Waveform, elapsed_ns: u64) -> Result<WaveformSample, AviateRuntimeError> {
    match waveform {
        // A step holds until the mission phase ends it. The waveform states
        // no window of its own, so it never completes on its own.
        Waveform::Step { value } => Ok(WaveformSample::Active(clamp_normalized("step", *value)?)),
        Waveform::Ramp {
            from,
            to,
            duration_ns,
        } => ramp(*from, *to, *duration_ns, elapsed_ns),
        Waveform::Pulse { value, duration_ns } => {
            if elapsed_ns >= *duration_ns {
                return Ok(WaveformSample::Complete);
            }
            Ok(WaveformSample::Active(clamp_normalized("pulse", *value)?))
        }
        Waveform::Reversal {
            first,
            second,
            dwell_ns,
        } => reversal(*first, *second, *dwell_ns, elapsed_ns),
        Waveform::Multisine {
            bias,
            components,
            duration_ns,
        } => multisine(*bias, components, *duration_ns, elapsed_ns),
        // A recorded artifact needs an artifact store that the vehicle
        // action port does not own. Refusing it here keeps a mission that
        // names one from silently flying a different stimulus.
        Waveform::Recorded { .. } => Err(AviateRuntimeError::UnsupportedWaveform {
            waveform: "recorded",
        }),
    }
}

fn ramp(
    from: f64,
    to: f64,
    duration_ns: u64,
    elapsed_ns: u64,
) -> Result<WaveformSample, AviateRuntimeError> {
    if elapsed_ns >= duration_ns {
        return Ok(WaveformSample::Complete);
    }
    let start = clamp_normalized("ramp start", from)?;
    let end = clamp_normalized("ramp end", to)?;
    let fraction = crate::runtime::math::progress(elapsed_ns, duration_ns);
    clamp_normalized("ramp value", (end - start).mul_add(fraction, start))
        .map(WaveformSample::Active)
}

fn reversal(
    first: f64,
    second: f64,
    dwell_ns: u64,
    elapsed_ns: u64,
) -> Result<WaveformSample, AviateRuntimeError> {
    let total_ns = dwell_ns
        .checked_mul(2)
        .ok_or(AviateRuntimeError::InvalidValue {
            field: "reversal duration",
        })?;
    if elapsed_ns >= total_ns {
        return Ok(WaveformSample::Complete);
    }
    let value = if elapsed_ns < dwell_ns { first } else { second };
    clamp_normalized("reversal value", value).map(WaveformSample::Active)
}

fn multisine(
    bias: f64,
    components: &[SineComponent],
    duration_ns: u64,
    elapsed_ns: u64,
) -> Result<WaveformSample, AviateRuntimeError> {
    if elapsed_ns >= duration_ns {
        return Ok(WaveformSample::Complete);
    }
    let seconds = crate::runtime::math::seconds(elapsed_ns);
    let mut total = require_finite("multisine bias", bias)?;
    for component in components {
        let sum = total + component_value(component, seconds)?;
        total = require_finite("multisine sum", sum)?;
    }
    clamp_normalized("multisine value", total).map(WaveformSample::Active)
}

fn component_value(component: &SineComponent, seconds: f64) -> Result<f64, AviateRuntimeError> {
    let amplitude = require_finite("multisine amplitude", component.amplitude)?;
    let frequency = require_finite("multisine frequency", component.frequency_hz)?;
    let phase = require_finite("multisine phase", component.phase_rad)?;
    let angle = require_finite(
        "multisine angle",
        std::f64::consts::TAU * frequency * seconds + phase,
    )?;
    require_finite("multisine component", amplitude * angle.sin())
}
