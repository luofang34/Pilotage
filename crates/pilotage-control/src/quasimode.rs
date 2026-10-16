//! The gimbal quasimode, ported from the retired JS prototype and expressed
//! as pure functions over a profile's bindings. LT held redirects the right
//! stick from flight to `vehicle.gimbal` line-of-sight rate demands; while it
//! is held the right stick and LT read neutral to every flight scheme, so no
//! scheme can consume a captured input by accident. R3 recenters the gimbal.

use pilotage_input::normalize_axis;

use crate::profile::GimbalDoc;
use crate::sample::RawSample;

/// The gimbal line-of-sight rate demand: pitch and yaw in `[-1, 1]`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct GimbalDemand {
    pub(crate) pitch: f32,
    pub(crate) yaw: f32,
}

/// Whether the gimbal quasimode is engaged: the modifier button (LT) is held,
/// analog past half travel or reported pressed.
pub(crate) fn modifier_held(sample: &RawSample, gimbal: &GimbalDoc) -> bool {
    let index = usize::from(gimbal.modifier_button);
    sample.button_value(index) > 0.5 || sample.pressed(index)
}

/// The right-stick gimbal rates under the quasimode, shaped through the
/// profile's per-axis curve (deadzone/expo/invert/calibration). Invert lives
/// in the profile: the default inverts pitch so stick-up is camera-up.
pub(crate) fn gimbal_demand(sample: &RawSample, gimbal: &GimbalDoc) -> GimbalDemand {
    let pitch = normalize_axis(sample.axis(gimbal.pitch.source_index), &gimbal.pitch);
    let yaw = normalize_axis(sample.axis(gimbal.yaw.source_index), &gimbal.yaw);
    GimbalDemand {
        pitch: pitch.value,
        yaw: yaw.value,
    }
}

/// Whether the recenter (R3) button reads pressed this tick.
pub(crate) fn reset_held(sample: &RawSample, gimbal: &GimbalDoc) -> bool {
    sample.pressed(usize::from(gimbal.reset_button))
}

/// The outcome of one R3 edge-tracking tick. The baseline ALWAYS advances to
/// the current held state, so an R3 held across a lease grant, reconnect, or
/// mode switch never reads as a fresh rising edge the moment the gimbal path
/// re-activates. The edge fires only on a genuine press transition while the
/// path is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResetEdge {
    pub(crate) edge: bool,
    pub(crate) baseline: bool,
}

/// Advances the R3 recenter baseline and reports whether an edge fired.
pub(crate) fn reset_edge(reset_held: bool, prev_held: bool, active: bool) -> ResetEdge {
    ResetEdge {
        edge: active && reset_held && !prev_held,
        baseline: reset_held,
    }
}

/// One tick's active gimbal frame decision. `None` means no active frame —
/// the caller still sends an idle zero-rate frame while the lease is held,
/// because a continuous stream is the scope's liveness. Otherwise carries the
/// rates, a one-shot recenter, and whether to keep streaming; an exit from
/// the quasimode yields exactly one trailing neutral frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FramePlan {
    pub(crate) pitch: f32,
    pub(crate) yaw: f32,
    pub(crate) recenter: bool,
    pub(crate) streaming: bool,
}

/// Decides this tick's gimbal frame, given whether the quasimode is held, the
/// recenter edge, whether a stream was in progress, and the shaped rates.
pub(crate) fn frame_plan(
    held: bool,
    reset_edge: bool,
    streaming: bool,
    demand: GimbalDemand,
) -> Option<FramePlan> {
    if !held && !reset_edge && !streaming {
        return None;
    }
    let (pitch, yaw) = if held {
        (demand.pitch, demand.yaw)
    } else {
        (0.0, 0.0)
    };
    Some(FramePlan {
        pitch,
        yaw,
        recenter: reset_edge,
        streaming: held,
    })
}

#[cfg(test)]
mod tests;
