//! Raw device sample: the input to the normalization pipeline (ADR-0007).

use pilotage_timing::MonoTimestamp;

/// A single physical-device sample, as reported by a platform port before any
/// normalization, calibration, or logical binding is applied.
///
/// `axes` are raw units exactly as reported by the underlying device API
/// (e.g. a browser Gamepad API axis value, or a HID report field); they are
/// not assumed to already lie in `[-1.0, 1.0]`. `buttons` is a bitmask with
/// bit `n` set when button `n` is currently held.
#[derive(Debug, Clone, PartialEq)]
pub struct RawDeviceSample {
    /// Raw axis values in device-reported units, indexed by source index.
    pub axes: Vec<f32>,
    /// Bitmask of currently held buttons, indexed by source index.
    pub buttons: u64,
    /// Monotonic timestamp at which this sample was captured.
    pub sampled_at: MonoTimestamp,
}

impl RawDeviceSample {
    /// Constructs a raw device sample from its constituent fields.
    #[must_use]
    pub const fn new(axes: Vec<f32>, buttons: u64, sampled_at: MonoTimestamp) -> Self {
        Self {
            axes,
            buttons,
            sampled_at,
        }
    }

    /// Returns whether source button index `index` is held in this sample.
    ///
    /// Indexes `>= 64` never read as held, since the mask has only 64 bits.
    #[must_use]
    pub const fn button_held(&self, index: u8) -> bool {
        if index >= 64 {
            return false;
        }
        (self.buttons & (1u64 << index)) != 0
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::RawDeviceSample;
    use pilotage_timing::MonoTimestamp;

    #[test]
    fn button_held_reads_correct_bit() {
        let sample = RawDeviceSample::new(vec![], 0b1010, MonoTimestamp::from_nanos(0));
        assert!(!sample.button_held(0));
        assert!(sample.button_held(1));
        assert!(!sample.button_held(2));
        assert!(sample.button_held(3));
    }

    #[test]
    fn button_held_out_of_range_index_is_false() {
        let sample = RawDeviceSample::new(vec![], u64::MAX, MonoTimestamp::from_nanos(0));
        assert!(!sample.button_held(64));
        assert!(!sample.button_held(255));
    }
}
