//! WASM resources for the shared feeder (#252): the browser's telemetry
//! ingress, trackers, and display-profile conversions delegate to the
//! portable runtime so client script holds no wire- or
//! measurement-interpreting logic (ADR-0029, ADR-0032). The script
//! wrappers keep only decode-shape validation and marshalling; every
//! semantic judgement runs in `indicate-instrument-feeder` behind the
//! runtime's plain-Rust feeder state.

use pilotage_instrument_runtime::feeder::{FcState, Ingress, IngressParams, NavGuidance, Turn};
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
    inner: Ingress,
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
            inner: Ingress::new(&IngressParams {
                vehicle_id,
                source_id,
                incarnation,
                sim_accept_unseen,
                maximum_seen_incarnations,
                maximum_skew_nanos,
            }),
        })
    }

    /// Ingests one decoded publication; returns whether admitted state
    /// changed.
    pub fn ingest(&mut self, sample: JsValue, now_ms: f64) -> Result<bool, JsValue> {
        let sample: JsSample = serde_wasm_bindgen::from_value(sample)
            .map_err(|error| to_js_error("avionics sample", error))?;
        let sample: indicate_instrument_feeder::avionics::AvionicsSample = sample
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
    inner: Turn,
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
        let raw: Option<indicate_instrument_feeder::RawStamp> = match stamp {
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
    inner: FcState,
}

#[wasm_bindgen]
impl FeederFcState {
    /// A tracker with the given staleness threshold in milliseconds.
    #[wasm_bindgen(constructor)]
    pub fn new(stale_after_ms: f64) -> Self {
        Self {
            inner: FcState::new(stale_after_ms),
        }
    }

    /// Feeds one decoded report (or `null`) and returns the view.
    pub fn observe(&mut self, report: JsValue, now_ms: f64) -> Result<JsValue, JsValue> {
        let report: Option<JsFcReport> = serde_wasm_bindgen::from_value(report)
            .map_err(|error| to_js_error("fc report", error))?;
        let report: Option<indicate_instrument_feeder::fc_state::FcReport> = match report {
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
    inner: NavGuidance,
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
        let converted = sample.map(JsGuidanceSample::into_lane);
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
    match pilotage_instrument_runtime::feeder::nav_display_state(snapshot.as_ref()) {
        Some(stamped) => serialize(&feeder_shapes::JsNavGroup::from(stamped)),
        None => Ok(JsValue::NULL),
    }
}

/// The semantic stamp-fault vocabulary in the script's `{field, rule}`
/// language, packed as `"field:rule"`; the wrapper splits it so both
/// sides speak one reason vocabulary.
#[wasm_bindgen]
pub fn feeder_stamp_fault(role: u8, clock: u8, integrity: u8, lane_role: u8) -> Option<String> {
    pilotage_instrument_runtime::feeder::stamp_fault(role, clock, integrity, lane_role)
        .map(|(field, rule)| format!("{field}:{rule}"))
}
