//! Layer controls and source-state policy.

use std::collections::BTreeMap;

/// Stable identity of the terrain base layer.
pub const TERRAIN_LAYER_ID: &str = "terrain-base";
/// Stable identity of the traffic layer.
pub const TRAFFIC_LAYER_ID: &str = "traffic";
/// Stable identity of the weather report layer.
pub const WEATHER_REPORT_LAYER_ID: &str = "weather-reports";
/// Stable identity of the weather advisory layer.
pub const WEATHER_ADVISORY_LAYER_ID: &str = "weather-advisories";

/// State of one display source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerSourceState {
    /// The source supplies current values.
    Live,
    /// The source exists, but it does not supply current values.
    Stale,
    /// The source is not available.
    Absent,
}

impl LayerSourceState {
    fn label(self) -> &'static str {
        match self {
            Self::Live => "Live",
            Self::Stale => "Stale",
            Self::Absent => "Absent",
        }
    }
}

/// One user-controlled display layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayerControl {
    /// Stable application identity.
    pub id: String,
    /// User-facing layer name.
    pub title: String,
    /// Whether the layer is visible.
    pub enabled: bool,
    /// Current source state.
    pub source_state: LayerSourceState,
    /// User-facing source state.
    pub source_state_label: String,
    /// Explanation of the source state.
    pub source_detail: String,
}

/// A radio band that can supply situation data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadioBand {
    /// The 1090 MHz ADS-B band.
    Adsb1090,
    /// The 978 MHz UAT band.
    Uat978,
}

/// A raw reception state from the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadioReceptionState {
    /// The host checks the source.
    Checking,
    /// The host does not have permission to use the source.
    PermissionDenied,
    /// The user disabled the source.
    DriverDisabled,
    /// No receiver is attached.
    Unplugged,
    /// A receiver is ready.
    Ready,
    /// A receiver supplies data.
    Streaming,
    /// The host stopped reception.
    Suspended,
    /// The source does not have sufficient power.
    Underpowered,
    /// Source enumeration failed.
    EnumerationFailure,
    /// A source endpoint failed.
    EndpointFailure,
    /// The source was removed.
    DeviceRemoved,
}

/// State of one attached radio receiver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RadioReceiverObservation {
    /// Receiver band.
    pub band: RadioBand,
    /// Receiver state.
    pub state: RadioReceptionState,
}

/// Raw source facts supplied by the composition host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceObservation {
    /// Whether the terrain archive is available.
    pub terrain_available: bool,
    /// Whether weather station positions are available.
    pub weather_positions_available: bool,
    /// State of the radio subsystem.
    pub radio_state: RadioReceptionState,
    /// States of attached receivers.
    pub radio_receivers: Vec<RadioReceiverObservation>,
}

impl Default for SourceObservation {
    fn default() -> Self {
        Self {
            terrain_available: false,
            weather_positions_available: false,
            radio_state: RadioReceptionState::Suspended,
            radio_receivers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LayerPolicy {
    enabled: BTreeMap<&'static str, bool>,
    sources: SourceObservation,
}

impl Default for LayerPolicy {
    fn default() -> Self {
        Self {
            enabled: [
                (TERRAIN_LAYER_ID, true),
                (TRAFFIC_LAYER_ID, true),
                (WEATHER_REPORT_LAYER_ID, true),
                (WEATHER_ADVISORY_LAYER_ID, true),
            ]
            .into_iter()
            .collect(),
            sources: SourceObservation::default(),
        }
    }
}

impl LayerPolicy {
    pub(crate) fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        let Some(value) = self.enabled.get_mut(id) else {
            return false;
        };
        *value = enabled;
        true
    }

    pub(crate) fn is_enabled(&self, id: &str) -> bool {
        self.enabled.get(id).copied().unwrap_or(false)
    }

    pub(crate) fn observe_sources(&mut self, sources: SourceObservation) {
        self.sources = sources;
    }

    pub(crate) fn controls(&self, has_traffic: bool, has_weather: bool) -> Vec<LayerControl> {
        vec![
            self.terrain_control(),
            self.traffic_control(has_traffic),
            self.weather_control(has_weather),
            self.advisory_control(),
        ]
    }

    fn terrain_control(&self) -> LayerControl {
        if self.sources.terrain_available {
            self.control(
                TERRAIN_LAYER_ID,
                "Terrain base",
                LayerSourceState::Live,
                "The bundled terrain source is available.",
            )
        } else {
            self.control(
                TERRAIN_LAYER_ID,
                "Terrain base",
                LayerSourceState::Absent,
                "The bundled terrain source is not available.",
            )
        }
    }

    fn traffic_control(&self, has_traffic: bool) -> LayerControl {
        let states = self.relevant_states(|_| true);
        if states
            .iter()
            .any(|state| **state == RadioReceptionState::Streaming)
        {
            return self.control(
                TRAFFIC_LAYER_ID,
                "Traffic",
                LayerSourceState::Live,
                &self.live_band_detail(),
            );
        }
        if has_traffic || states.iter().any(|state| source_is_present(**state)) {
            return self.control(
                TRAFFIC_LAYER_ID,
                "Traffic",
                LayerSourceState::Stale,
                "Traffic reception is not live. Retained tracks can be old.",
            );
        }
        self.control(
            TRAFFIC_LAYER_ID,
            "Traffic",
            LayerSourceState::Absent,
            "No traffic source is available.",
        )
    }

    fn weather_control(&self, has_weather: bool) -> LayerControl {
        if !self.sources.weather_positions_available {
            return self.control(
                WEATHER_REPORT_LAYER_ID,
                "Weather reports",
                LayerSourceState::Absent,
                "Weather station positions are not available. This state does not mean clear weather.",
            );
        }
        let states = self.relevant_states(|band| band == RadioBand::Uat978);
        if states
            .iter()
            .any(|state| **state == RadioReceptionState::Streaming)
        {
            return self.control(
                WEATHER_REPORT_LAYER_ID,
                "Weather reports",
                LayerSourceState::Live,
                "978 MHz weather reception is live.",
            );
        }
        if has_weather || states.iter().any(|state| source_is_present(**state)) {
            return self.control(
                WEATHER_REPORT_LAYER_ID,
                "Weather reports",
                LayerSourceState::Stale,
                "Weather reception is not live. Retained reports can be old.",
            );
        }
        self.control(
            WEATHER_REPORT_LAYER_ID,
            "Weather reports",
            LayerSourceState::Absent,
            "No weather report source is available. This state does not mean clear weather.",
        )
    }

    fn advisory_control(&self) -> LayerControl {
        self.control(
            WEATHER_ADVISORY_LAYER_ID,
            "Weather advisories",
            LayerSourceState::Absent,
            "No weather advisory source is available. This state does not mean clear weather.",
        )
    }

    fn control(
        &self,
        id: &'static str,
        title: &'static str,
        source_state: LayerSourceState,
        source_detail: &str,
    ) -> LayerControl {
        LayerControl {
            id: id.into(),
            title: title.into(),
            enabled: self.is_enabled(id),
            source_state,
            source_state_label: source_state.label().into(),
            source_detail: source_detail.into(),
        }
    }

    fn relevant_states(&self, include: impl Fn(RadioBand) -> bool) -> Vec<&RadioReceptionState> {
        let mut states: Vec<_> = self
            .sources
            .radio_receivers
            .iter()
            .filter(|receiver| include(receiver.band))
            .map(|receiver| &receiver.state)
            .collect();
        if self.sources.radio_receivers.is_empty() {
            states.push(&self.sources.radio_state);
        }
        states
    }

    fn live_band_detail(&self) -> String {
        let live_1090 = self.sources.radio_receivers.iter().any(|receiver| {
            receiver.band == RadioBand::Adsb1090 && receiver.state == RadioReceptionState::Streaming
        });
        let live_978 = self.sources.radio_receivers.iter().any(|receiver| {
            receiver.band == RadioBand::Uat978 && receiver.state == RadioReceptionState::Streaming
        });
        match (live_1090, live_978) {
            (true, true) => "1090 MHz and 978 MHz traffic reception are live.".into(),
            (true, false) => "1090 MHz traffic reception is live.".into(),
            (false, true) => "978 MHz traffic reception is live.".into(),
            (false, false) => "Traffic reception is live.".into(),
        }
    }
}

fn source_is_present(state: RadioReceptionState) -> bool {
    matches!(
        state,
        RadioReceptionState::Checking | RadioReceptionState::Ready | RadioReceptionState::Streaming
    )
}
