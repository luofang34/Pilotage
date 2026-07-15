//! Ground-truth tests for bounded conformal projection (HUD-03). This module
//! holds the shared deterministic fixtures; the tests live in its submodules:
//!
//! - [`projection`] — the projection/interpolation geometry, cross-checked
//!   against an independent f64 rotation-matrix oracle;
//! - [`verdict`] — the four-valued state machine and its structural, budget, and
//!   identity gates;
//! - [`kinematics`] — the kinematic-input validity gate (attitude, frame,
//!   provenance, and velocity-accuracy regressions).
//!
//! Every test is deterministic and synchronizes on values, never on time.
#![allow(clippy::expect_used, clippy::panic)]

mod kinematics;
mod projection;
mod verdict;

use pilotage_frames::{ClockDomain, Epoch, FrameId, Quat, Tagged, TimeScale};

use pilotage_camera_calibration::{
    SIM_FPV_CALIBRATION_HASH, SIM_FPV_CALIBRATION_ID, VerifiedCameraModel, sim_fpv_calibration,
};

use super::{
    AvailabilityInputs, Bracket, CaptureContext, ConformalFix, ConformalPolicy, KinematicSample,
    VelocityQuality, ViewGeometry, assess_conformal,
};
use crate::availability::{AvailabilityProfile, ExternalHealth, InputHealth};
use crate::datum::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
};
use crate::identity::{
    AttitudeQuality, CoherentSnapshot, IntegrityLevel, PositionQuality, SourceIncarnation,
    SourceStamp, StatedAttitude, StatedPosition,
};
use crate::view::{
    CalibrationId, CalibrationRef, MinificationPolicy, NearFarPolicy, Projection, ProjectionView,
};

const DEG: f64 = core::f64::consts::PI / 180.0;

// --- fixtures (shared with the `verdict` submodule via `use super::*`) ------

fn epoch(nanos: u64) -> Epoch {
    Epoch {
        clock: ClockDomain::Simulation,
        scale: TimeScale::Monotonic,
        nanos,
    }
}

fn stamp(nanos: u64, snap_id: u64) -> SourceStamp {
    SourceStamp {
        source_id: 7,
        incarnation: SourceIncarnation([9; 16]),
        generation: 1,
        sequence: (nanos & 0xffff_ffff) as u32,
        acquired_at: epoch(nanos),
        integrity: IntegrityLevel::Trusted,
        snapshot: CoherentSnapshot {
            producer: SourceIncarnation([5; 16]),
            generation: 1,
            id: snap_id,
        },
    }
}

fn geopos() -> GeodeticPosition {
    let vertical = VerticalPosition::new(
        300.0,
        VerticalDatum::Ellipsoid,
        GeoidModelId::UNDECLARED,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect("ellipsoidal height needs no extra identity");
    GeodeticPosition::new(
        37.0,
        -122.0,
        HorizontalDatum::Wgs84,
        DatumRealizationId::UNDECLARED,
        vertical,
    )
    .expect("a WGS-84 position is well-formed")
}

/// A sample whose attitude and position share one coherent snapshot stamp (zero
/// skew), with small, conformal-grade accuracies. The velocity is NED and the
/// body rate body-frame, each carrying the same coherent snapshot stamp as the
/// pose so the kinematic provenance is coherent.
fn sample(att: Quat, vel: [f64; 3], rate: [f32; 3], nanos: u64, snap_id: u64) -> KinematicSample {
    let s = stamp(nanos, snap_id);
    KinematicSample {
        attitude: StatedAttitude {
            attitude: att,
            stamp: s,
            quality: AttitudeQuality { angular_mrad: 2 },
        },
        position: StatedPosition {
            position: geopos(),
            stamp: s,
            quality: PositionQuality {
                horizontal_mm: 100,
                vertical_mm: 100,
            },
        },
        velocity: Tagged {
            frame: FrameId::Ned,
            epoch: epoch(nanos),
            meta: s,
            value: vel,
        },
        body_rate: Tagged {
            frame: FrameId::Body,
            epoch: epoch(nanos),
            meta: s,
            value: rate,
        },
        velocity_quality: VelocityQuality { sigma_mmps: 50 },
    }
}

/// A steady two-sample bracket 1 ms apart at the same pose/velocity/rate, ids
/// declared and in order.
fn steady(att: Quat, vel: [f64; 3], rate: [f32; 3]) -> Bracket {
    Bracket {
        older: sample(att, vel, rate, 1_000, 1),
        newer: sample(att, vel, rate, 2_000, 2),
    }
}

/// Aerospace ZYX (roll, pitch, yaw) body→NED quaternion, matching the presentation
/// convention used across the workspace.
fn euler_quat(roll_deg: f32, pitch_deg: f32, yaw_deg: f32) -> Quat {
    let d = core::f32::consts::PI / 180.0;
    let (r, p, y) = (roll_deg * d / 2.0, pitch_deg * d / 2.0, yaw_deg * d / 2.0);
    let (cr, sr) = (libm::cosf(r), libm::sinf(r));
    let (cp, sp) = (libm::cosf(p), libm::sinf(p));
    let (cy, sy) = (libm::cosf(y), libm::sinf(y));
    Quat {
        w: cr * cp * cy + sr * sp * sy,
        x: sr * cp * cy - cr * sp * sy,
        y: cr * sp * cy + sr * cp * sy,
        z: cr * cp * sy - sr * sp * cy,
    }
}

/// A time inside the published sim FPV calibration's effective window.
pub(super) const NOW_IN_WINDOW_NS: u64 = 1_600_000_000_000_000_000;

/// The calibration reference the sim model — and therefore the projection view —
/// is bound to (the published sim FPV identity and its content hash).
pub(super) fn calibration() -> CalibrationRef {
    CalibrationRef {
        calibration_id: CalibrationId(SIM_FPV_CALIBRATION_ID),
        content_hash: SIM_FPV_CALIBRATION_HASH,
    }
}

/// A genuinely hash-verified camera model, minted the only way one can be — from
/// the published sim calibration through its verify-and-mint path. The sim camera
/// is forward-looking (body FRD → OpenCV optical).
pub(super) fn verified_model() -> VerifiedCameraModel {
    sim_fpv_calibration()
        .verified_camera_model(SIM_FPV_CALIBRATION_HASH, NOW_IN_WINDOW_NS)
        .expect("the published sim calibration verifies and mints")
}

/// The resolved geometry, obtained the only way it can be — by deriving it from a
/// genuinely verified camera model.
fn geom() -> ViewGeometry {
    ViewGeometry::derive(&verified_model()).expect("the verified model derives geometry")
}

fn view() -> ProjectionView {
    ProjectionView {
        calibration: calibration(),
        projection: Projection::Perspective,
        near_far: NearFarPolicy {
            near_m: 0.5,
            far_m: 5000.0,
        },
        minification: MinificationPolicy::Trilinear,
    }
}

fn capture(nanos: u64) -> CaptureContext {
    CaptureContext {
        capture_epoch: epoch(nanos),
        clock_error_ns: 100_000,
        pipeline_latency_ns: 2_000_000,
    }
}

fn sim() -> ConformalPolicy {
    ConformalPolicy::simulator()
}

/// A policy whose valid/limited budget comfortably covers the published sim
/// calibration's static alignment bound (~0.0117 rad, which itself exceeds the
/// tight simulator valid threshold). Used where a test isolates the "genuine
/// verified calibration → Valid" or the availability-decides path from the
/// error budget.
pub(super) fn generous() -> ConformalPolicy {
    ConformalPolicy::new(
        crate::ConformalPolicyId(7),
        1,
        50_000_000,
        10_000_000,
        1.0,
        0.050,
        0.100,
        50.0,
    )
    .expect("a monotonic policy is valid")
}

/// External health with every producer-stated input nominal, so the derived
/// availability is decided by the samples' own integrity and accuracy.
pub(super) fn healthy_external() -> ExternalHealth {
    ExternalHealth {
        integrity: InputHealth::Ok,
        calibration: InputHealth::Ok,
        database: InputHealth::Ok,
        coverage: InputHealth::Ok,
        renderer: InputHealth::Ok,
    }
}

/// Availability inputs that let the samples decide: nominal external health and
/// the simulator profile.
pub(super) fn availability() -> AvailabilityInputs {
    AvailabilityInputs {
        external: healthy_external(),
        profile: AvailabilityProfile::simulator(),
    }
}

/// Assess with the nominal view/camera, sample-decided availability, and no path
/// cues.
fn run(bracket: &Bracket, cap: CaptureContext, policy: &ConformalPolicy) -> ConformalFix {
    assess_conformal(bracket, cap, &view(), &geom(), availability(), policy, &[])
}

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}
