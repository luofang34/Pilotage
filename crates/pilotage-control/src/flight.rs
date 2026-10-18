//! Flight-scheme mapping for Standard Gamepad pads, ported from the JS
//! schemes. Each mode commands the identical velocity control law; only the
//! stick assignment differs. While the gimbal quasimode is engaged the
//! captured inputs (the right stick and the modifier trigger) read neutral to
//! flight here, so a captured input can never be double-consumed.

use pilotage_input::{AxisConfig, normalize_axis};

use crate::profile::FlightDoc;
use crate::sample::{Mode, RawSample};

/// What the gimbal quasimode has captured this tick, so flight reads those
/// inputs as neutral. `active` is false when LT is not held.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Capture {
    pub(crate) active: bool,
    pub(crate) pitch_axis: usize,
    pub(crate) yaw_axis: usize,
    pub(crate) modifier_button: usize,
}

impl Capture {
    fn masks_axis(&self, index: usize) -> bool {
        self.active && (index == self.pitch_axis || index == self.yaw_axis)
    }

    fn masks_button(&self, index: usize) -> bool {
        self.active && index == self.modifier_button
    }
}

/// The four flight axis demands plus a readout label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlightAxes {
    pub(crate) roll: f32,
    pub(crate) pitch: f32,
    pub(crate) throttle: f32,
    pub(crate) yaw: f32,
    pub(crate) label: &'static str,
}

/// Maps a masked sample to flight axes under `mode`, shaping each stick axis
/// through the profile's precompiled stick config (no per-tick allocation).
pub(crate) fn flight_axes(
    sample: &RawSample,
    flight: &FlightDoc,
    stick: &AxisConfig,
    mode: Mode,
    capture: Capture,
) -> FlightAxes {
    let shaped = |index: usize| -> f32 {
        if capture.masks_axis(index) {
            return 0.0;
        }
        shaped_stick(sample, stick, index)
    };
    let trigger = |index: usize| -> f32 {
        if capture.masks_button(index) {
            0.0
        } else {
            sample.button_value(index).clamp(0.0, 1.0)
        }
    };
    let (left_x, left_y) = (shaped(flight.left_x), shaped(flight.left_y));
    let (right_x, right_y) = (shaped(flight.right_x), shaped(flight.right_y));
    match mode {
        Mode::QuadPilot => FlightAxes {
            roll: right_x,
            pitch: -right_y,
            throttle: -left_y,
            yaw: left_x,
            label: "PILOT (Mode 2): L=climb/yaw R=move",
        },
        Mode::Fpv => FlightAxes {
            roll: right_x,
            pitch: -right_y,
            throttle: -left_y,
            yaw: left_x,
            label: "FPV: R=tilt angle L=thrust/yaw",
        },
        Mode::QuadCruise => FlightAxes {
            roll: left_x,
            pitch: -left_y,
            throttle: (trigger(flight.trigger_right) - trigger(flight.trigger_left))
                .clamp(-1.0, 1.0),
            yaw: right_x,
            label: "CRUISE: L=move RX=yaw R2/L2=climb",
        },
        // Rover drives ground: throttle and yaw only; roll/pitch stay zero.
        Mode::Rover => FlightAxes {
            roll: 0.0,
            pitch: 0.0,
            throttle: -left_y,
            yaw: left_x,
            label: "ROVER: throttle/yaw",
        },
    }
}

/// The shaped value of one flight stick axis through the precompiled config,
/// shared by the scheme mapping and the activation neutrality check.
pub(crate) fn shaped_stick(sample: &RawSample, stick: &AxisConfig, index: usize) -> f32 {
    normalize_axis(sample.axis(index), stick).value
}
