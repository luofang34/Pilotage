//! Boxes (flight modes): their identifiers, the active-mode bits, and the
//! aux-channel ranges that switch them.

use super::PayloadError;

/// Permanent box identifiers (`msp_box.c`) an adapter switches.
pub mod permanent_id {
    /// ARM.
    pub const ARM: u8 = 0;
    /// ANGLE: self-levelling with the stick commanding a tilt angle.
    pub const ANGLE: u8 = 1;
    /// HORIZON.
    pub const HORIZON: u8 = 2;
    /// ALTHOLD.
    pub const ALTHOLD: u8 = 3;
    /// POS HOLD.
    pub const POSHOLD: u8 = 11;
    /// FAILSAFE.
    pub const FAILSAFE: u8 = 27;
    /// GPS RESCUE.
    pub const GPS_RESCUE: u8 = 46;
}

/// The lowest PWM value a mode range step describes, microseconds.
const STEP_ORIGIN_US: u16 = 900;
/// Microseconds per mode range step.
const STEP_US: u16 = 25;
/// The highest mode range step.
pub const MAX_STEP: u8 = 48;

/// The box list from `MSP_BOXIDS`: the permanent identifier of each box
/// in the order the status mode bits use.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BoxIds(pub Vec<u8>);

impl BoxIds {
    /// Decodes an `MSP_BOXIDS` payload: one permanent identifier per byte.
    #[must_use]
    pub fn decode(payload: &[u8]) -> Self {
        Self(payload.to_vec())
    }

    /// Whether the box with `permanent_id` is active in `modes`. A box the
    /// firmware does not list is never active.
    #[must_use]
    pub fn is_active(&self, modes: ModeFlags, permanent_id: u8) -> bool {
        self.0
            .iter()
            .position(|&id| id == permanent_id)
            .is_some_and(|index| modes.bit(index))
    }
}

/// Active-mode bits from the status reply, indexed by [`BoxIds`] position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModeFlags(pub u128);

impl ModeFlags {
    /// Whether the bit at `index` is set.
    #[must_use]
    pub fn bit(self, index: usize) -> bool {
        u32::try_from(index)
            .ok()
            .and_then(|shift| self.0.checked_shr(shift))
            .is_some_and(|shifted| shifted & 1 != 0)
    }
}

/// One mode activation condition: a box is active while the aux channel
/// is inside `[start_step, end_step]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeRange {
    /// The box's permanent identifier.
    pub permanent_id: u8,
    /// Aux channel index; 0 is the first aux channel (RC channel 5).
    pub aux_channel: u8,
    /// First step of the active range (`900 + 25 * step` µs).
    pub start_step: u8,
    /// Last step of the active range.
    pub end_step: u8,
}

impl ModeRange {
    /// A range over `[start_us, end_us]`, rounded to the step grid and
    /// clamped to the steps Betaflight accepts.
    #[must_use]
    pub fn from_pulse(permanent_id: u8, aux_channel: u8, start_us: u16, end_us: u16) -> Self {
        let step = |us: u16| {
            let steps = us.saturating_sub(STEP_ORIGIN_US) / STEP_US;
            u8::try_from(steps).unwrap_or(MAX_STEP).min(MAX_STEP)
        };
        Self {
            permanent_id,
            aux_channel,
            start_step: step(start_us),
            end_step: step(end_us),
        }
    }

    /// Whether the range is unused. Betaflight reports an unused slot as
    /// equal start and end steps.
    #[must_use]
    pub fn is_unused(&self) -> bool {
        self.start_step >= self.end_step
    }

    /// Whether a pulse of `us` on this range's channel activates the box.
    #[must_use]
    pub fn contains(&self, us: u16) -> bool {
        let lo = STEP_ORIGIN_US + STEP_US * u16::from(self.start_step);
        let hi = STEP_ORIGIN_US + STEP_US * u16::from(self.end_step);
        !self.is_unused() && (lo..hi).contains(&us)
    }

    /// Decodes an `MSP_MODE_RANGES` payload: four bytes per slot. A
    /// trailing partial slot is ignored.
    #[must_use]
    pub fn decode_all(payload: &[u8]) -> Vec<Self> {
        payload
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[permanent_id, aux_channel, start_step, end_step]| Self {
                permanent_id,
                aux_channel,
                start_step,
                end_step,
            })
            .collect()
    }
}

/// Encodes an `MSP_SET_MODE_RANGE` payload for slot `index`.
///
/// # Errors
///
/// [`PayloadError::TooMany`] when a step is above [`MAX_STEP`].
pub fn encode_set_mode_range(index: u8, range: ModeRange) -> Result<Vec<u8>, PayloadError> {
    for step in [range.start_step, range.end_step] {
        if step > MAX_STEP {
            return Err(PayloadError::TooMany {
                message: "MSP_SET_MODE_RANGE",
                what: "range steps",
                count: usize::from(step),
                limit: usize::from(MAX_STEP),
            });
        }
    }
    Ok(vec![
        index,
        range.permanent_id,
        range.aux_channel,
        range.start_step,
        range.end_step,
    ])
}
