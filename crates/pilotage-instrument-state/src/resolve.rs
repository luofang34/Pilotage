//! Resolution from raw input state to display-ready signals.

use libm::{atan2f, sqrtf};

use crate::aircraft::{
    AircraftState, EstimateQuality, NavData, Selections, SnapshotCoherence, Wind,
};
use crate::signal::{FreshnessPolicy, Sig, SignalStatus};
use crate::units::{M_TO_FT, MPS_TO_FPM, MPS_TO_KT};

/// Below this groundspeed the track angle is geometrically meaningless
/// and resolves `Missing` instead of jittering.
const TRACK_MIN_GS_MPS: f32 = 0.5;

/// Resolved navigation guidance for the HSI.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NavResolved {
    /// Guidance data as provided; `source == None` removes the CDI.
    pub data: NavData,
    /// Status of the guidance group as a whole.
    pub status: SignalStatus,
}

/// Display-ready state consumed by every panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelData {
    /// Bank angle, radians, positive right.
    pub roll_rad: Sig<f32>,
    /// Pitch angle, radians, positive nose-up.
    pub pitch_rad: Sig<f32>,
    /// Heading, radians clockwise from north.
    pub heading_rad: Sig<f32>,
    /// Body yaw rate, radians/second (turn-rate proxy).
    pub turn_rate_rps: Sig<f32>,
    /// Indicated airspeed, knots.
    pub ias_kt: Sig<f32>,
    /// Groundspeed, knots.
    pub gs_kt: Sig<f32>,
    /// Altitude above the local origin, feet.
    pub alt_ft: Sig<f32>,
    /// Vertical speed, feet/minute, positive climbing.
    pub vsi_fpm: Sig<f32>,
    /// Ground track, radians clockwise from north.
    pub track_rad: Sig<f32>,
    /// Altimeter setting, hectopascals.
    pub baro_hpa: Sig<f32>,
    /// Wind estimate.
    pub wind: Sig<Wind>,
    /// Navigation guidance.
    pub nav: NavResolved,
    /// Pilot selections, passed through untouched.
    pub selections: Selections,
}

fn quality_status(q: EstimateQuality) -> SignalStatus {
    match q {
        EstimateQuality::Good => SignalStatus::Valid,
        EstimateQuality::Degraded => SignalStatus::Degraded,
        EstimateQuality::Unusable => SignalStatus::Failed,
    }
}

fn flag_status(valid: bool) -> SignalStatus {
    if valid {
        SignalStatus::Valid
    } else {
        SignalStatus::Failed
    }
}

/// Attitude and kinematics are stamped independently; when the ingress
/// gate reports their acquisition times exceed the skew budget, each
/// value is individually usable but the pair must not present as one
/// coherent aircraft state, so both groups degrade (amber, value shown).
/// `Insufficient` means too few stamped groups to judge — the ordinary
/// missing/freshness handling already covers that case.
fn coherence_status(coherence: SnapshotCoherence) -> SignalStatus {
    match coherence {
        SnapshotCoherence::ExcessiveSkew => SignalStatus::Degraded,
        SnapshotCoherence::Insufficient | SnapshotCoherence::Coherent => SignalStatus::Valid,
    }
}

/// Resolves raw input state into display-ready signals.
///
/// Each signal's status is the worst of: its group's freshness under
/// `policy`, the source quality, the snapshot's group-coherence result,
/// and the source's validity flag for that group. Values behind
/// `Missing`/`Failed` are quiet zeros a panel never paints.
pub fn resolve(state: &AircraftState, policy: &FreshnessPolicy) -> PanelData {
    let quality = quality_status(state.quality);
    let coherence = coherence_status(state.snapshot.coherence);

    let att_fresh = if state.attitude.data.is_none() {
        SignalStatus::Missing
    } else {
        policy.status_for_age(state.attitude.age_ms)
    };
    let att_status = att_fresh
        .worst(quality)
        .worst(coherence)
        .worst(flag_status(state.valid.attitude));
    let rate_status = att_fresh
        .worst(quality)
        .worst(coherence)
        .worst(flag_status(state.valid.rates));
    let (roll, pitch, yaw, turn_rate) = match state.attitude.data {
        Some(att) => {
            let (r, p, y) = att.quat.to_euler();
            (r, p, y, att.rates_rps[2])
        }
        None => (0.0, 0.0, 0.0, 0.0),
    };

    let kin_fresh = if state.kinematics.data.is_none() {
        SignalStatus::Missing
    } else {
        policy.status_for_age(state.kinematics.age_ms)
    };
    let pos_status = kin_fresh
        .worst(quality)
        .worst(coherence)
        .worst(flag_status(state.valid.position));
    let vel_status = kin_fresh
        .worst(quality)
        .worst(coherence)
        .worst(flag_status(state.valid.velocity));
    let (alt_ft, vsi_fpm, gs_kt, track_rad, gs_mps) = match state.kinematics.data {
        Some(kin) => {
            let alt = -kin.pos_ned_m[2] * M_TO_FT;
            let vsi = -kin.vel_ned_mps[2] * MPS_TO_FPM;
            let gs_mps = sqrtf(
                kin.vel_ned_mps[0] * kin.vel_ned_mps[0] + kin.vel_ned_mps[1] * kin.vel_ned_mps[1],
            );
            let track = atan2f(kin.vel_ned_mps[1], kin.vel_ned_mps[0]);
            (alt, vsi, gs_mps * MPS_TO_KT, track, gs_mps)
        }
        None => (0.0, 0.0, 0.0, 0.0, 0.0),
    };
    let track_status = if gs_mps < TRACK_MIN_GS_MPS {
        SignalStatus::Missing
    } else {
        vel_status
    };

    let air_fresh = policy.status_for_age(state.air.age_ms);
    let air = state.air.data.unwrap_or_default();
    let ias = match air.ias_mps {
        Some(v) => Sig::with_status(v * MPS_TO_KT, air_fresh.worst(quality)),
        None => Sig::missing(),
    };
    let baro = match air.baro_setting_hpa {
        Some(v) => Sig::with_status(v, air_fresh),
        None => Sig::missing(),
    };

    let nav_fresh = policy.status_for_age(state.nav.age_ms);
    let nav = match state.nav.data {
        Some(data) => NavResolved {
            data,
            status: nav_fresh,
        },
        None => NavResolved::default(),
    };

    let wind = match (state.wind.data, policy.status_for_age(state.wind.age_ms)) {
        (Some(w), s) if s.shows_value() => Sig::with_status(w, s),
        _ => Sig::with_status(
            Wind {
                from_rad: 0.0,
                speed_mps: 0.0,
            },
            SignalStatus::Missing,
        ),
    };

    PanelData {
        roll_rad: Sig::with_status(roll, att_status),
        pitch_rad: Sig::with_status(pitch, att_status),
        heading_rad: Sig::with_status(yaw, att_status),
        turn_rate_rps: Sig::with_status(turn_rate, rate_status),
        ias_kt: ias,
        gs_kt: Sig::with_status(gs_kt, vel_status),
        alt_ft: Sig::with_status(alt_ft, pos_status),
        vsi_fpm: Sig::with_status(vsi_fpm, vel_status),
        track_rad: Sig::with_status(track_rad, track_status),
        baro_hpa: baro,
        wind,
        nav,
        selections: state.selections,
    }
}

#[cfg(test)]
mod tests;
