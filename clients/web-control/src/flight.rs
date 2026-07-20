//! Flight-scheme mapping for Standard Gamepad pads, ported from the JS
//! schemes. Each mode commands the identical velocity control law; only the
//! stick assignment differs. While the gimbal quasimode is engaged the
//! captured inputs (the right stick and the modifier trigger) read neutral to
//! flight here, so a captured input can never be double-consumed.

use pilotage_input::{AxisCalibration, AxisConfig, normalize_axis};

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

/// Maps a masked sample to flight axes under `mode`, applying the profile's
/// shared deadzone/expo shaping to each stick axis.
pub(crate) fn flight_axes(
    sample: &RawSample,
    flight: &FlightDoc,
    mode: Mode,
    capture: Capture,
) -> FlightAxes {
    let shaped = |index: usize| -> f32 {
        if capture.masks_axis(index) {
            return 0.0;
        }
        normalize_axis(sample.axis(index), &stick_config(index, flight)).value
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

/// Builds the shared shaping config for one flight stick axis from the
/// profile's flight deadzone/expo. Flight sticks carry no inversion of their
/// own — the scheme applies the sign — and use the identity calibration.
fn stick_config(index: usize, flight: &FlightDoc) -> AxisConfig {
    AxisConfig {
        source_index: index,
        logical: "roll".to_string(),
        invert: false,
        deadzone: flight.deadzone,
        expo: flight.expo,
        calibration: AxisCalibration {
            min: -1.0,
            center: 0.0,
            max: 1.0,
        },
    }
}
