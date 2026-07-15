//! Bounded conformal projection (HUD-03): align the coherent aircraft state to
//! video capture time and project registered symbology **only** while a bounded
//! total-error budget holds.
//!
//! This module consumes the crate's existing contracts and adds no parallel
//! ones. It interpolates the [`crate::CoherentSnapshot`]-identified state at
//! capture time (attitude by normalized quaternion interpolation), combines a
//! dynamic [`AlignmentErrorBound`] from named, individually sourced terms,
//! projects the horizon, flight-path marker, and runway/path cues through the
//! referenced camera model of a [`crate::ProjectionView`], and emits a
//! four-valued [`ConformalState`] (`Valid` / `Limited` / `NonConformal` /
//! `Unavailable`) that carries a typed [`ConformalReason`]. When the budget or a
//! registration limit is exceeded the registered cues are **removed**
//! ([`ConformalFix::cues`] is `None`), never left as plausible stale alignment.
//!
//! The resolved camera geometry ([`ViewGeometry`]) embeds the calibration it was
//! resolved from, and the assessment refuses a geometry whose calibration does
//! not match the projection view's — so a geometry or alignment bound resolved
//! from the wrong calibration can never yield a registered scene.
//!
//! # What it reuses (no parallel types)
//!
//! - camera model / projection: [`crate::ProjectionView`], [`crate::Projection`],
//!   [`crate::CalibrationRef`];
//! - availability: [`crate::SvsAvailability`] and its [`crate::AvailabilityReason`]
//!   are delegated into [`ConformalReason::Availability`];
//! - capture-time coherent state and identity: [`crate::StatedAttitude`],
//!   [`crate::StatedPosition`], [`crate::SourceStamp`], [`crate::CoherentSnapshot`],
//!   [`crate::SourceIncarnation`]. The capture-clock mapping itself is the
//!   caller's (the adapter's `CaptureClockMapping`): the caller maps the capture
//!   time into the flight-state clock and supplies it as [`CaptureContext`], so
//!   this crate does not re-mint a capture-time type.
//!
//! SIM / NOT FOR FLIGHT.

use pilotage_frames::{AngularVelocity, Epoch, FrameId, ROTATION_NORM_TOLERANCE, Velocity};

use crate::availability::{AvailabilityProfile, ExternalHealth, SvsAvailability, derive_inputs};
use crate::identity::{
    CoherentSnapshot, SourceIncarnation, SourceStamp, StatedAttitude, StatedPosition,
};
use crate::view::{Projection, ProjectionView};

mod budget;
mod interp;
mod policy;
mod project;
mod state;

pub use budget::AlignmentErrorBound;
pub use policy::{
    ConformalError, ConformalPolicy, ConformalPolicyId, SIMULATOR_CONFORMAL_POLICY_ID,
};
pub use project::{
    HorizonLine, ScreenMark, ViewGeometry, down_in_camera, project_flight_path, project_horizon,
    project_path_cue,
};
pub use state::{ConformalReason, ConformalState};

/// The maximum number of runway/path cue points one [`ConformalCues`] carries
/// inline. It covers a runway outline (four threshold/corner points) plus a few
/// approach-path or flight-path tunnel gates; a caller with more points projects
/// the remainder itself through [`project_path_cue`] under the same verdict. The
/// cap keeps the result allocation-free and `Copy` in this `no_std` contract.
pub const MAX_PATH_CUES: usize = 8;

/// A 1-sigma accuracy estimate for a velocity, in millimeters/second — the
/// velocity-error input to the alignment budget. A distinct type so a velocity
/// accuracy is never read as a position or attitude accuracy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VelocityQuality {
    /// 1-sigma speed accuracy, millimeters/second.
    pub sigma_mmps: u32,
}

/// One coherent kinematic sample: a pose (attitude + position, one coherent
/// snapshot) plus the velocity and body rate — each frame-, epoch-, and
/// snapshot-tagged, not a bare array. The velocity is NED (the flight-path-marker
/// reference) and the body rate is body-frame; [`assess_conformal`] enforces both
/// frames and that the velocity and rate share the pose's coherent snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KinematicSample {
    /// Body → NED attitude, with its identity stamp and angular accuracy.
    pub attitude: StatedAttitude,
    /// Geodetic position, with its identity stamp and position accuracy.
    pub position: StatedPosition,
    /// NED velocity, meters/second — frame must be [`FrameId::Ned`]; carries its
    /// own frame, epoch, and [`SourceStamp`] provenance.
    pub velocity: Velocity<SourceStamp>,
    /// Body-frame angular rate, radians/second — frame must be [`FrameId::Body`];
    /// carries its own frame, epoch, and [`SourceStamp`] provenance.
    pub body_rate: AngularVelocity<SourceStamp>,
    /// The velocity's 1-sigma speed accuracy (the velocity-error budget input).
    pub velocity_quality: VelocityQuality,
}

/// Two coherent samples that bracket the capture time. The capture time is
/// interpolated between them; outside `[older, newer]` the nearest endpoint is
/// used and the extrapolation distance is charged to the error budget and the
/// policy limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bracket {
    /// The earlier sample.
    pub older: KinematicSample,
    /// The later sample.
    pub newer: KinematicSample,
}

/// The timing facts of the capture the caller has already resolved: the capture
/// time expressed in the **flight-state clock** (the caller applied the adapter's
/// `CaptureClockMapping`), the clock-mapping error bound, and the measured
/// pipeline latency. Neither is a hidden constant; each is sourced by the caller.
/// The calibration's static alignment bound is **not** here — it is carried by
/// the derived [`ViewGeometry`], so it stays bound to the calibration it was
/// resolved from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureContext {
    /// Capture time, mapped into the aircraft state's clock domain and scale.
    pub capture_epoch: Epoch,
    /// Symmetric bound on the capture-clock mapping error, nanoseconds.
    pub clock_error_ns: u64,
    /// Measured glass-to-glass pipeline latency, nanoseconds.
    pub pipeline_latency_ns: u64,
}

/// The synthetic-vision availability inputs the conformal path derives its scene
/// verdict from: the producer-stated external subsystem health and the intended
/// availability profile. The verdict is **derived** from the actual bracket
/// samples against these — never taken as a caller-asserted value — so a low
/// integrity or accuracy in the samples cannot be masked by an optimistic claim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvailabilityInputs {
    /// Producer-stated health of the inputs the contract cannot check itself
    /// (integrity monitor, calibration, database, coverage, renderer).
    pub external: ExternalHealth,
    /// The intended-function availability profile (freshness/accuracy limits).
    pub profile: AvailabilityProfile,
}

/// The traceable identity a conformal fix carries, tying the **video** (the
/// capture time it was aligned to) and the **overlay** (the coherent snapshots and
/// source incarnation of the interpolated state) to a traceable identity — reusing
/// [`CoherentSnapshot`] and [`SourceIncarnation`], not a new identity type. The
/// interpolation reads **both** bracket endpoints, so both snapshot identities are
/// retained (an interpolated fix belongs to no single snapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixIdentity {
    /// The coherent snapshot of the older bracket endpoint.
    pub older_snapshot: CoherentSnapshot,
    /// The coherent snapshot of the newer bracket endpoint.
    pub newer_snapshot: CoherentSnapshot,
    /// The source incarnation both endpoints belong to (one continuous stream).
    pub source_incarnation: SourceIncarnation,
    /// The capture time the interpolated state was aligned to.
    pub capture_epoch: Epoch,
}

impl FixIdentity {
    fn at_capture(bracket: &Bracket, capture_epoch: Epoch) -> Self {
        Self {
            older_snapshot: bracket.older.attitude.stamp.snapshot,
            newer_snapshot: bracket.newer.attitude.stamp.snapshot,
            source_incarnation: bracket.newer.attitude.stamp.incarnation,
            capture_epoch,
        }
    }
}

/// The registered conformal cues for one frame. Present only when the state
/// [`ConformalState::draws_cues`]; a [`ConformalState::NonConformal`] or
/// [`ConformalState::Unavailable`] fix carries no cues at all, so an exceeded
/// budget removes every cue — horizon, flight-path marker, and runway/path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConformalCues {
    /// The horizon line in normalized image coordinates.
    pub horizon: HorizonLine,
    /// The flight-path marker, or `None` when the velocity has no in-front
    /// direction to draw.
    pub flight_path: Option<ScreenMark>,
    /// The projected runway/path cues, one slot per supplied point in order.
    /// Slots `[0, path_count)` are the supplied points — each `Some` when it
    /// projects in front of the camera and within the near/far clip, or `None`
    /// when it is behind or clipped; slots `[path_count, MAX_PATH_CUES)` are
    /// unused. An off-scale point is `Some` with `within_fov = false`.
    pub path: [Option<ScreenMark>; MAX_PATH_CUES],
    /// How many supplied runway/path points were considered (`≤ MAX_PATH_CUES`).
    pub path_count: usize,
    /// Whether the cues must be marked reduced-confidence
    /// ([`ConformalState::Limited`]).
    pub limited: bool,
}

/// The result of a conformal assessment: the verdict, the error breakdown (once
/// interpolation ran), the cues (only when drawable), and the traceable identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConformalFix {
    /// The registration verdict.
    pub state: ConformalState,
    /// The dynamic alignment-error breakdown, or `None` when the fix was refused
    /// structurally before any interpolation (invalid view, unavailable scene, or
    /// a broken identity).
    pub error: Option<AlignmentErrorBound>,
    /// The registered cues, present only when [`ConformalState::draws_cues`].
    pub cues: Option<ConformalCues>,
    /// The traceable frame/snapshot identity.
    pub identity: FixIdentity,
}

impl ConformalFix {
    fn refused(state: ConformalState, identity: FixIdentity) -> Self {
        Self {
            state,
            error: None,
            cues: None,
            identity,
        }
    }
}

/// Assesses whether the coherent state may be projected conformally at the
/// capture time, and projects the cues when it may.
///
/// Precedence (fail-closed): an invalid view or an unavailable scene is
/// `Unavailable`; a broken capture/identity relationship is `NonConformal`; then
/// the interpolation, the dynamic error budget, and the policy limits decide
/// `NonConformal` / `Limited` / `Valid` by fixed precedence. Cues are drawn only
/// for a drawable verdict, so an exceeded budget removes them.
#[must_use]
pub fn assess_conformal(
    bracket: &Bracket,
    capture: CaptureContext,
    view: &ProjectionView,
    geom: &ViewGeometry,
    availability: AvailabilityInputs,
    policy: &ConformalPolicy,
    path_ned_m: &[[f64; 3]],
) -> ConformalFix {
    let identity = FixIdentity::at_capture(bracket, capture.capture_epoch);

    if view.validate().is_err()
        || !matches!(view.projection, Projection::Perspective)
        || geom.validate().is_err()
    {
        return ConformalFix::refused(
            ConformalState::Unavailable(ConformalReason::ViewInvalid),
            identity,
        );
    }
    // The resolved geometry (extrinsics, field of view, alignment bound) must come
    // from the same calibration the view references, or the projection would run
    // through the wrong camera model and still register.
    if geom.calibration != view.calibration {
        return ConformalFix::refused(
            ConformalState::Unavailable(ConformalReason::CalibrationMismatch),
            identity,
        );
    }
    if let Some(reason) = identity_fault(bracket, capture.capture_epoch) {
        return ConformalFix::refused(ConformalState::NonConformal(reason), identity);
    }
    // Both endpoint attitudes must be unit rotations, and the velocity/rate must be
    // in their expected frames and share the pose's coherent snapshot — a zero
    // quaternion or a mis-framed/mis-provenanced kinematic input never registers.
    if let Some(reason) = kinematic_fault(bracket) {
        return ConformalFix::refused(ConformalState::NonConformal(reason), identity);
    }
    // Availability is DERIVED from the actual samples (integrity + accuracy)
    // against the profile — never taken as a caller-asserted verdict — so a low
    // integrity or accuracy in the samples cannot be masked by an optimistic claim.
    let scene = derive_availability(bracket, availability, capture.capture_epoch);
    if let SvsAvailability::Unavailable(reason) = scene {
        return ConformalFix::refused(
            ConformalState::Unavailable(ConformalReason::Availability(reason)),
            identity,
        );
    }

    let interp = interp::interpolate(&bracket.older, &bracket.newer, capture.capture_epoch.nanos);
    let error = budget::compute(
        geom.alignment_bound_rad,
        capture.clock_error_ns,
        capture.pipeline_latency_ns,
        &interp,
        policy,
    );
    let state = state::classify(
        interp.timing.skew_ns,
        interp.timing.extrapolation_ns,
        rate_magnitude(interp.body_rate_rps),
        &error,
        scene,
        policy,
    );
    let cues = state
        .draws_cues()
        .then(|| build_cues(&interp, view, geom, path_ned_m, !state.is_valid()));
    ConformalFix {
        state,
        error: Some(error),
        cues,
        identity,
    }
}

/// Derives the synthetic-vision availability from the ACTUAL bracket samples —
/// each endpoint's integrity and accuracy against the profile — taking the worse
/// of the two. An untrusted or low-accuracy sample yields Unavailable or Degraded
/// no matter what a caller claims.
///
/// Freshness is judged against the later of the capture time and the newer
/// endpoint, so that a bracketed capture time (which precedes the newer sample)
/// never flags that sample as a future reading; the conformal path bounds how far
/// the capture may sit outside the bracket separately, through the extrapolation
/// limit. By this point [`identity_fault`] has already established that both
/// samples and the capture share one clock and scale.
fn derive_availability(
    bracket: &Bracket,
    availability: AvailabilityInputs,
    capture: Epoch,
) -> SvsAvailability {
    let newer_at = bracket.newer.attitude.stamp.acquired_at;
    let reference = if capture.nanos >= newer_at.nanos {
        capture
    } else {
        newer_at
    };
    let assess = |s: &KinematicSample| {
        SvsAvailability::assess(&derive_inputs(
            &s.position,
            &s.attitude,
            &availability.external,
            reference,
            &availability.profile,
        ))
    };
    worst_availability(assess(&bracket.older), assess(&bracket.newer))
}

/// The worse of two availability verdicts (Unavailable worse than Degraded worse
/// than Available).
fn worst_availability(a: SvsAvailability, b: SvsAvailability) -> SvsAvailability {
    match (a, b) {
        (SvsAvailability::Unavailable(r), _) | (_, SvsAvailability::Unavailable(r)) => {
            SvsAvailability::Unavailable(r)
        }
        (SvsAvailability::Degraded(r), _) | (_, SvsAvailability::Degraded(r)) => {
            SvsAvailability::Degraded(r)
        }
        _ => SvsAvailability::Available,
    }
}

/// The kinematic-validity fault of a bracket: a non-unit attitude quaternion at
/// either endpoint, a velocity or body rate in the wrong frame, or a velocity/rate
/// whose provenance is not the pose's coherent snapshot. `None` when both samples
/// are well-formed.
fn kinematic_fault(bracket: &Bracket) -> Option<ConformalReason> {
    for s in [&bracket.older, &bracket.newer] {
        if s.attitude
            .attitude
            .renormalized(ROTATION_NORM_TOLERANCE)
            .is_err()
        {
            return Some(ConformalReason::AttitudeNotARotation);
        }
        if s.velocity.frame != FrameId::Ned || s.body_rate.frame != FrameId::Body {
            return Some(ConformalReason::KinematicFrame);
        }
        if !s.attitude.stamp.coherent_with(&s.velocity.meta)
            || !s.attitude.stamp.coherent_with(&s.body_rate.meta)
        {
            return Some(ConformalReason::KinematicProvenance);
        }
    }
    None
}

/// Projects every cue for a drawable fix: the horizon, the flight-path marker,
/// and the runway/path points (up to [`MAX_PATH_CUES`], each clipped to the
/// view's near/far policy). Only called for a drawable verdict, so a non-drawable
/// fix carries no cues at all.
fn build_cues(
    interp: &interp::Interpolated,
    view: &ProjectionView,
    geom: &ViewGeometry,
    path_ned_m: &[[f64; 3]],
    limited: bool,
) -> ConformalCues {
    let mut path = [None; MAX_PATH_CUES];
    let path_count = path_ned_m.len().min(MAX_PATH_CUES);
    for (slot, point) in path.iter_mut().zip(path_ned_m.iter()).take(path_count) {
        *slot = project::project_path_cue(interp.attitude, *point, view.near_far, geom);
    }
    ConformalCues {
        horizon: project::project_horizon(interp.attitude, geom),
        flight_path: project::project_flight_path(interp.attitude, interp.velocity_ned_mps, geom),
        path,
        path_count,
        limited,
    }
}

/// The structural fault that makes the capture/state relationship untrustworthy,
/// or `None` when the bracket is a single coherent, in-order stream the capture
/// time is expressed against.
fn identity_fault(bracket: &Bracket, capture: Epoch) -> Option<ConformalReason> {
    let older = bracket.older.attitude.stamp;
    let newer = bracket.newer.attitude.stamp;
    if !bracket
        .older
        .attitude
        .stamp
        .coherent_with(&bracket.older.position.stamp)
        || !bracket
            .newer
            .attitude
            .stamp
            .coherent_with(&bracket.newer.position.stamp)
    {
        return Some(ConformalReason::SnapshotIncoherent);
    }
    if !same_stream(&older, &newer) || older.acquired_at.nanos > newer.acquired_at.nanos {
        return Some(ConformalReason::StreamDiscontinuity);
    }
    if capture.clock != newer.acquired_at.clock || capture.scale != newer.acquired_at.scale {
        return Some(ConformalReason::ClockIncoherent);
    }
    None
}

/// Whether two stamps are one continuous source stream: same source, incarnation,
/// boot/attachment generation, and sampling clock and scale. Distinct from
/// coherence, which additionally binds a single snapshot instance and so never
/// holds across two different-time samples.
fn same_stream(a: &SourceStamp, b: &SourceStamp) -> bool {
    a.source_id == b.source_id
        && a.incarnation == b.incarnation
        && a.generation == b.generation
        && a.acquired_at.clock == b.acquired_at.clock
        && a.acquired_at.scale == b.acquired_at.scale
}

fn rate_magnitude(rate_rps: [f32; 3]) -> f64 {
    let (x, y, z) = (
        f64::from(rate_rps[0]),
        f64::from(rate_rps[1]),
        f64::from(rate_rps[2]),
    );
    libm::sqrt(x * x + y * y + z * z)
}

#[cfg(test)]
mod tests;
