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
use indicate_instrument_feeder::nav_guidance::{
    Guidance, NavCounters, NavGuidanceTracker, NavReject, NavSnapshot,
};
use indicate_instrument_feeder::turn::{TurnDeclaration, TurnDerivation};
use indicate_instrument_feeder::{RawStamp, StampFault};
use indicate_instrument_state::{NavData, Stamped};

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
        self.inner.observe(sample, now_ms)
    }

    /// The current snapshot without feeding a sample.
    pub fn snapshot(&self, now_ms: f64) -> Option<NavSnapshot> {
        self.inner.snapshot(now_ms)
    }

    /// Acceptance/refusal counters plus the last refusal reason.
    pub fn diagnostics(&self) -> (NavCounters, Option<NavReject>) {
        self.inner.diagnostics()
    }
}

/// Converts one accepted guidance snapshot into the instrument model's
/// nav group, or `None` when guidance must not display (ADR-0031).
pub fn nav_display_state(snapshot: Option<&NavSnapshot>) -> Option<Stamped<NavData>> {
    indicate_instrument_feeder::nav_display::nav_display_state(snapshot)
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
