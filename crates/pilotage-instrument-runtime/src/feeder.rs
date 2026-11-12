//! Platform-neutral feeder state (ADR-0032): the construction, ingest,
//! snapshot, and diagnostics wiring of the shared feeder lanes over
//! `indicate-instrument-feeder`, with plain Rust in/out types. Shells
//! marshal these values across their boundary; they hold no wire- or
//! measurement-interpreting logic of their own (ADR-0029).

use indicate_instrument_feeder::avionics::{
    AvionicsIngress, AvionicsSample, IncarnationPolicy, IngressConfig, IngressCounters,
    IngressSnapshot,
};
use indicate_instrument_feeder::fc_state::{FcReport, FcStateTracker, FcView};
use indicate_instrument_feeder::nav_display::{DeviationScaleProfile, PhaseWidths};
use indicate_instrument_feeder::nav_guidance::{
    Guidance, NavCounters, NavGuidanceTracker, NavReject, NavSnapshot,
};
use indicate_instrument_feeder::turn::{TurnDeclaration, TurnDerivation};
use indicate_instrument_feeder::{ROLE_OPERATIONAL_ESTIMATE, RawStamp, StampFault};
use indicate_instrument_state::{NavData, NavScale, Stamped};

/// Construction parameters for [`Ingress`], in plain Rust.
pub struct IngressParams {
    /// The vehicle this ingress admits publications for.
    pub vehicle_id: u64,
    /// The pinned source, or `None` to accept the pin policy's choice.
    pub source_id: Option<u64>,
    /// The pinned 16-byte incarnation identity, or `None` for
    /// pin-on-first-sight.
    pub incarnation: Option<[u8; 16]>,
    /// Selects the simulation incarnation policy (accept unseen)
    /// instead of pin-first.
    pub sim_accept_unseen: bool,
    /// Maximum tracked incarnations before the oldest is dropped.
    pub maximum_seen_incarnations: u32,
    /// Maximum admitted skew between grouped stamps, in nanoseconds.
    pub maximum_skew_nanos: u64,
}

/// The AV-01 avionics ingress as plain Rust state.
pub struct Ingress {
    inner: AvionicsIngress,
}

impl Ingress {
    /// Creates an ingress from plain parameters.
    pub fn new(params: &IngressParams) -> Self {
        Self {
            inner: AvionicsIngress::new(IngressConfig {
                vehicle_id: params.vehicle_id,
                role: ROLE_OPERATIONAL_ESTIMATE,
                source_id: params.source_id,
                incarnation: params.incarnation,
                incarnation_policy: if params.sim_accept_unseen {
                    IncarnationPolicy::SimAcceptUnseen
                } else {
                    IncarnationPolicy::PinFirst
                },
                maximum_seen_incarnations: params.maximum_seen_incarnations as usize,
                maximum_skew_nanos: params.maximum_skew_nanos,
            }),
        }
    }

    /// Ingests one decoded publication; returns whether admitted state
    /// changed.
    pub fn ingest(&mut self, sample: &AvionicsSample, now_ms: f64) -> bool {
        self.inner.ingest(sample, now_ms)
    }

    /// The current admitted state, aged against the caller's clock.
    pub fn snapshot(&self, now_ms: f64) -> IngressSnapshot {
        self.inner.snapshot(now_ms)
    }

    /// Refusal counters and the last stamp fault.
    pub fn diagnostics(&self) -> (IngressCounters, Option<StampFault>) {
        self.inner.diagnostics()
    }
}

/// DYN-01 turn derivation as plain Rust state.
#[derive(Default)]
pub struct Turn {
    inner: TurnDerivation,
}

impl Turn {
    /// A derivation that has seen nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears all state.
    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// Consumes the current declared heading with its stamp; returns a
    /// dynamics declaration or `None`.
    pub fn update(
        &mut self,
        heading_rad: f64,
        age_ms: f64,
        stamp: Option<&RawStamp>,
    ) -> Option<TurnDeclaration> {
        self.inner.update(heading_rad, age_ms, stamp)
    }
}

/// The pinned FC-state lane as plain Rust state.
pub struct FcState {
    inner: FcStateTracker,
}

impl FcState {
    /// A tracker with the given staleness threshold in milliseconds.
    pub fn new(stale_after_ms: f64) -> Self {
        Self {
            inner: FcStateTracker::new(stale_after_ms),
        }
    }

    /// Feeds one decoded report (or `None`) and returns the view.
    pub fn observe(&mut self, report: Option<&FcReport>, now_ms: f64) -> Option<FcView> {
        self.inner.observe(report, now_ms)
    }
}

/// The pinned navigation-guidance lane as plain Rust state.
#[derive(Default)]
pub struct NavGuidance {
    inner: NavGuidanceTracker,
    last_flight_phase: u8,
    last_rnp_m: Option<f32>,
}

impl NavGuidance {
    /// A tracker that has seen nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one decoded guidance sample (or `None`) and returns the
    /// snapshot.
    pub fn observe(
        &mut self,
        sample: Option<&(RawStamp, Guidance)>,
        now_ms: f64,
    ) -> Option<NavSnapshot> {
        let phase = sample.map_or(0, |(_, guidance)| match guidance.scale {
            NavScale::Enroute => 1,
            NavScale::Terminal => 2,
            NavScale::Approach => 3,
            NavScale::Unknown => 0,
        });
        self.observe_with_phase_rnp(sample, phase, None, now_ms)
    }

    /// Feeds one decoded guidance sample with its wire flight-phase code
    /// (`1..=4`) and optional RNP in meters, updating the retained phase
    /// and RNP only when the tracker accepts a new sample.
    pub fn observe_with_phase_rnp(
        &mut self,
        sample: Option<&(RawStamp, Guidance)>,
        flight_phase: u8,
        rnp_m: Option<f32>,
        now_ms: f64,
    ) -> Option<NavSnapshot> {
        let accepted_before = self.inner.diagnostics().0.accepted;
        let out = self.inner.observe(sample, now_ms);
        if self.inner.diagnostics().0.accepted != accepted_before {
            self.last_flight_phase = flight_phase;
            self.last_rnp_m = rnp_m.filter(|value| value.is_finite() && *value > 0.0);
        }
        out
    }

    /// The current snapshot without feeding a sample.
    pub fn snapshot(&self, now_ms: f64) -> Option<NavSnapshot> {
        self.inner.snapshot(now_ms)
    }

    /// The wire flight-phase code (`1..=4`, or `0` unstated) of the most
    /// recently accepted sample.
    #[must_use]
    pub const fn last_flight_phase(&self) -> u8 {
        self.last_flight_phase
    }

    /// The RNP in meters of the most recently accepted sample, when
    /// positive and finite.
    #[must_use]
    pub const fn last_rnp_m(&self) -> Option<f32> {
        self.last_rnp_m
    }

    /// Acceptance/refusal counters plus the last refusal reason.
    pub fn diagnostics(&self) -> (NavCounters, Option<NavReject>) {
        self.inner.diagnostics()
    }
}

/// Maps the wire's `flight_phase` (`1` enroute, `2` terminal, `3` approach,
/// `4` missed approach) into the instrument model's [`NavScale`]; `0` or
/// any unknown code maps to [`NavScale::Unknown`], which fail-closes the
/// display.
#[must_use]
pub const fn nav_scale_from_wire_phase(flight_phase: u32) -> NavScale {
    match flight_phase {
        1 => NavScale::Enroute,
        2 | 4 => NavScale::Terminal,
        3 => NavScale::Approach,
        _ => NavScale::Unknown,
    }
}

fn tightened_profile(base: DeviationScaleProfile, rnp_m: Option<f32>) -> DeviationScaleProfile {
    let Some(rnp) = rnp_m.filter(|value| value.is_finite() && *value > 0.0) else {
        return base;
    };
    let tighten = |widths: PhaseWidths| PhaseWidths {
        lateral_m: widths.lateral_m.min(rnp),
        vertical_m: widths.vertical_m,
    };
    DeviationScaleProfile {
        enroute: tighten(base.enroute),
        terminal: tighten(base.terminal),
        approach: tighten(base.approach),
    }
}

/// Converts one accepted guidance snapshot into the instrument model's
/// nav group, or `None` when guidance must not display (ADR-0031). The
/// deviations are scaled for the small unmanned airframes Pilotage flies:
/// 200 m lateral full scale enroute, 50 m terminal, 20 m approach, and
/// 16 m vertical.
pub fn nav_display_state(snapshot: Option<&NavSnapshot>) -> Option<Stamped<NavData>> {
    nav_display_state_with_rnp(snapshot, None)
}

/// Converts one accepted guidance snapshot into the instrument model's
/// nav group using Pilotage's small-unmanned profile, tightening full-scale
/// lateral deflection when `rnp_m` specifies a tighter RNP than the phase
/// baseline.
pub fn nav_display_state_with_rnp(
    snapshot: Option<&NavSnapshot>,
    rnp_m: Option<f32>,
) -> Option<Stamped<NavData>> {
    let profile = tightened_profile(DeviationScaleProfile::SMALL_UNMANNED, rnp_m);
    indicate_instrument_feeder::nav_display::nav_display_state(snapshot, &profile)
}

/// Converts one accepted guidance snapshot into the instrument model's
/// nav group using the RTCA DO-229 GNSS navigator profile (`2.0 NM`
/// enroute, `1.0 NM` terminal/missed, `0.3 NM` approach), tightening
/// full-scale lateral deflection when `rnp_m` specifies a tighter
/// procedure RNP.
pub fn gnss_navigator_display_state(
    snapshot: Option<&NavSnapshot>,
    rnp_m: Option<f32>,
) -> Option<Stamped<NavData>> {
    let profile = tightened_profile(DeviationScaleProfile::GNSS_NAVIGATOR, rnp_m);
    indicate_instrument_feeder::nav_display::nav_display_state(snapshot, &profile)
}

/// The semantic stamp-fault vocabulary as a `{field, rule}` pair, so
/// every shell speaks one reason vocabulary; shells only format it.
pub fn stamp_fault(
    role: u8,
    clock: u8,
    integrity: u8,
    lane_role: u8,
) -> Option<(&'static str, &'static str)> {
    let probe = RawStamp {
        role,
        integrity,
        source_id: 0,
        incarnation: [0; 16],
        epoch: 0,
        sequence: 0,
        acquired_at_ns: 0,
        clock,
    };
    indicate_instrument_feeder::stamp_fault_for_role(&probe, lane_role).map(|fault| match fault {
        StampFault::RoleMismatch => ("role", "role-mismatch"),
        StampFault::IllegalClock => ("clock", "malformed"),
        StampFault::UnknownIntegrity => ("integrity", "unknown"),
    })
}
