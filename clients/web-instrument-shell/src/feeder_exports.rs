//! WASM resources for the shared feeder (#252): the browser's telemetry
//! ingress, trackers, and display-profile conversions delegate here so
//! client script holds no wire- or measurement-interpreting logic
//! (ADR-0029). The script wrappers keep only decode-shape validation
//! and marshalling; every semantic judgement runs in
//! `indicate-instrument-feeder`, the same crate a native shell links.

use indicate_instrument_feeder::avionics::{
    AvionicsIngress, AvionicsSample, IncarnationPolicy, IngressConfig,
};
use indicate_instrument_feeder::fc_state::{FcReport, FcStateTracker};
use indicate_instrument_feeder::nav_display::nav_display_state;
use indicate_instrument_feeder::nav_guidance::{Guidance, NavGuidanceTracker};
use indicate_instrument_feeder::turn::TurnDerivation;
use indicate_instrument_feeder::{RawStamp, StampFault};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

mod feeder_shapes;
use feeder_shapes::{JsFcReport, JsGuidanceSample, JsSample, JsStamp, serialize, snapshot_js};

fn to_js_error(context: &str, error: impl core::fmt::Display) -> JsValue {
    JsValue::from_str(&format!("{context}: {error}"))
}

/// The AV-01 avionics ingress as a JavaScript-owned resource.
#[wasm_bindgen]
pub struct FeederIngress {
    inner: AvionicsIngress,
}

#[wasm_bindgen]
impl FeederIngress {
    /// Creates an ingress. `incarnation` is the 32-hex pinned identity
    /// or empty for pin-on-first-sight; `sim_accept_unseen` selects the
    /// simulation incarnation policy.
    #[wasm_bindgen(constructor)]
    pub fn new(
        vehicle_id: u64,
        source_id: Option<u64>,
        incarnation_hex: String,
        sim_accept_unseen: bool,
        maximum_seen_incarnations: u32,
        maximum_skew_nanos: u64,
    ) -> Result<FeederIngress, JsValue> {
        let incarnation = if incarnation_hex.is_empty() {
            None
        } else {
            Some(
                feeder_shapes::parse_incarnation(&incarnation_hex)
                    .map_err(|error| to_js_error("sourceIncarnation", error))?,
            )
        };
        Ok(Self {
            inner: AvionicsIngress::new(IngressConfig {
                vehicle_id,
                source_id,
                incarnation,
                incarnation_policy: if sim_accept_unseen {
                    IncarnationPolicy::SimAcceptUnseen
                } else {
                    IncarnationPolicy::PinFirst
                },
                maximum_seen_incarnations: maximum_seen_incarnations as usize,
                maximum_skew_nanos,
            }),
        })
    }

    /// Ingests one decoded publication; returns whether admitted state
    /// changed.
    pub fn ingest(&mut self, sample: JsValue, now_ms: f64) -> Result<bool, JsValue> {
        let sample: JsSample = serde_wasm_bindgen::from_value(sample)
            .map_err(|error| to_js_error("avionics sample", error))?;
        let sample: AvionicsSample = sample
            .try_into()
            .map_err(|error: &str| to_js_error("avionics sample", error))?;
        Ok(self.inner.ingest(&sample, now_ms))
    }

    /// The current admitted state, ages against the caller's clock.
    pub fn snapshot(&self, now_ms: f64) -> Result<JsValue, JsValue> {
        snapshot_js(&self.inner.snapshot(now_ms))
    }

    /// Refusal counters; the wrapper merges its own decode-shape counts.
    pub fn diagnostics(&self) -> Result<JsValue, JsValue> {
        let (counters, _) = self.inner.diagnostics();
        serialize(&feeder_shapes::JsIngressCounters::from(counters))
    }
}

/// DYN-01 turn derivation as a JavaScript-owned resource.
#[wasm_bindgen]
#[derive(Default)]
pub struct FeederTurn {
    inner: TurnDerivation,
}

#[wasm_bindgen]
impl FeederTurn {
    /// A derivation that has seen nothing.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears all state.
    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// Consumes the current declared heading with its stamp; returns a
    /// dynamics declaration object or `null`.
    pub fn update(
        &mut self,
        heading_rad: f64,
        age_ms: f64,
        stamp: JsValue,
    ) -> Result<JsValue, JsValue> {
        let stamp: Option<JsStamp> = serde_wasm_bindgen::from_value(stamp)
            .map_err(|error| to_js_error("turn stamp", error))?;
        let raw: Option<RawStamp> = match stamp {
            Some(stamp) => Some(
                stamp
                    .try_into()
                    .map_err(|error: &str| to_js_error("turn stamp", error))?,
            ),
            None => None,
        };
        let declared = self.inner.update(heading_rad, age_ms, raw.as_ref());
        match declared {
            Some(declaration) => serialize(&feeder_shapes::JsTurnDeclaration::from(declaration)),
            None => Ok(JsValue::NULL),
        }
    }
}

/// The pinned FC-state lane as a JavaScript-owned resource.
#[wasm_bindgen]
pub struct FeederFcState {
    inner: FcStateTracker,
}

#[wasm_bindgen]
impl FeederFcState {
    /// A tracker with the given staleness threshold in milliseconds.
    #[wasm_bindgen(constructor)]
    pub fn new(stale_after_ms: f64) -> Self {
        Self {
            inner: FcStateTracker::new(stale_after_ms),
        }
    }

    /// Feeds one decoded report (or `null`) and returns the view.
    pub fn observe(&mut self, report: JsValue, now_ms: f64) -> Result<JsValue, JsValue> {
        let report: Option<JsFcReport> = serde_wasm_bindgen::from_value(report)
            .map_err(|error| to_js_error("fc report", error))?;
        let report: Option<FcReport> = match report {
            Some(report) => Some(
                report
                    .try_into()
                    .map_err(|error: &str| to_js_error("fc report", error))?,
            ),
            None => None,
        };
        match self.inner.observe(report.as_ref(), now_ms) {
            Some(view) => serialize(&feeder_shapes::JsFcView::from(view)),
            None => Ok(JsValue::NULL),
        }
    }
}

/// The pinned navigation-guidance lane as a JavaScript-owned resource.
#[wasm_bindgen]
#[derive(Default)]
pub struct FeederNavGuidance {
    inner: NavGuidanceTracker,
}

#[wasm_bindgen]
impl FeederNavGuidance {
    /// A tracker that has seen nothing.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one decoded guidance sample (or `null`) and returns the
    /// snapshot.
    pub fn observe(&mut self, sample: JsValue, now_ms: f64) -> Result<JsValue, JsValue> {
        let sample: Option<JsGuidanceSample> = serde_wasm_bindgen::from_value(sample)
            .map_err(|error| to_js_error("nav guidance", error))?;
        let converted: Option<(RawStamp, Guidance)> = sample.map(JsGuidanceSample::into_lane);
        match self.inner.observe(converted.as_ref(), now_ms) {
            Some(snapshot) => serialize(&feeder_shapes::JsNavSnapshot::from(snapshot)),
            None => Ok(JsValue::NULL),
        }
    }

    /// The current snapshot without feeding a sample.
    pub fn snapshot(&self, now_ms: f64) -> Result<JsValue, JsValue> {
        match self.inner.snapshot(now_ms) {
            Some(snapshot) => serialize(&feeder_shapes::JsNavSnapshot::from(snapshot)),
            None => Ok(JsValue::NULL),
        }
    }

    /// Acceptance/refusal counters plus the last refusal reason string.
    pub fn diagnostics(&self) -> Result<JsValue, JsValue> {
        let (counters, last) = self.inner.diagnostics();
        serialize(&feeder_shapes::JsNavDiagnostics::new(counters, last))
    }
}

/// Converts one accepted guidance snapshot into the instrument model's
/// nav group, or `null` when guidance must not display (ADR-0031).
#[wasm_bindgen]
pub fn feeder_nav_display_state(snapshot: JsValue) -> Result<JsValue, JsValue> {
    let snapshot: Option<feeder_shapes::JsNavSnapshotIn> = serde_wasm_bindgen::from_value(snapshot)
        .map_err(|error| to_js_error("nav snapshot", error))?;
    let snapshot = snapshot.map(feeder_shapes::JsNavSnapshotIn::into_snapshot);
    match nav_display_state(snapshot.as_ref()) {
        Some(stamped) => serialize(&feeder_shapes::JsNavGroup::from(stamped)),
        None => Ok(JsValue::NULL),
    }
}

/// The semantic stamp-fault vocabulary in the script's `{field, rule}`
/// language, packed as `"field:rule"`; the wrapper splits it so both
/// sides speak one reason vocabulary.
#[wasm_bindgen]
pub fn feeder_stamp_fault(role: u8, clock: u8, integrity: u8, lane_role: u8) -> Option<String> {
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
    indicate_instrument_feeder::stamp_fault_for_role(&probe, lane_role).map(|fault| {
        let (field, rule) = match fault {
            StampFault::RoleMismatch => ("role", "role-mismatch"),
            StampFault::IllegalClock => ("clock", "malformed"),
            StampFault::UnknownIntegrity => ("integrity", "unknown"),
        };
        format!("{field}:{rule}")
    })
}
