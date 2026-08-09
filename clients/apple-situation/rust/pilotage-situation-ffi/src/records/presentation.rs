//! Layer, source, and traffic detail records.

/// State of one layer source.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum DisplayLayerSourceState {
    /// The source supplies current values.
    Live,
    /// The source exists, but it does not supply current values.
    Stale,
    /// The source is not available.
    Absent,
}

/// One user-controlled display layer.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct DisplayLayerControl {
    /// Stable application identity.
    pub id: String,
    /// User-facing layer name.
    pub title: String,
    /// Whether the layer is visible.
    pub enabled: bool,
    /// Current source state.
    pub source_state: DisplayLayerSourceState,
    /// User-facing source state.
    pub source_state_label: String,
    /// Explanation of the source state.
    pub source_detail: String,
}

/// A radio band observed by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum PresentationRadioBand {
    /// The 1090 MHz ADS-B band.
    Adsb1090,
    /// The 978 MHz UAT band.
    Uat978,
}

/// A raw radio state observed by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum PresentationRadioState {
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

/// State of one receiver observed by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Record)]
pub struct PresentationReceiverObservation {
    /// Receiver band.
    pub band: PresentationRadioBand,
    /// Receiver state.
    pub state: PresentationRadioState,
}

/// Raw source facts observed by the host.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct PresentationSourceObservation {
    /// Whether the terrain archive is available.
    pub terrain_available: bool,
    /// State of the radio subsystem.
    pub radio_state: PresentationRadioState,
    /// States of attached receivers.
    pub radio_receivers: Vec<PresentationReceiverObservation>,
}

impl Default for PresentationSourceObservation {
    fn default() -> Self {
        Self {
            terrain_available: false,
            radio_state: PresentationRadioState::Suspended,
            radio_receivers: Vec::new(),
        }
    }
}

impl PresentationSourceObservation {
    pub(crate) fn into_portable(
        self,
        weather_positions_available: bool,
    ) -> pilotage_presentation::SourceObservation {
        pilotage_presentation::SourceObservation {
            terrain_available: self.terrain_available,
            weather_positions_available,
            radio_state: self.radio_state.into(),
            radio_receivers: self.radio_receivers.into_iter().map(Into::into).collect(),
        }
    }
}

/// One traffic item that has no map position.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct DisplayTrafficListItem {
    /// Stable display identity.
    pub id: String,
    /// Primary display text.
    pub title: String,
    /// Ready-to-display lifecycle summary.
    pub summary: String,
}

/// One traffic field and its evidence.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct DisplayTrafficDetailField {
    /// Stable field identity.
    pub id: String,
    /// User-facing field name.
    pub title: String,
    /// Ready-to-display value.
    pub value: Option<String>,
    /// Ready-to-display age.
    pub age: Option<String>,
    /// Ready-to-display source.
    pub source: Option<String>,
    /// Reason that the value is absent.
    pub absence_reason: Option<String>,
}

/// Complete display detail for one retained traffic track.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct DisplayTrafficDetail {
    /// Stable display identity.
    pub id: String,
    /// Primary display text.
    pub title: String,
    /// Primary over-the-air identity.
    pub primary_identity: String,
    /// Other associated over-the-air identities.
    pub other_identities: Vec<String>,
    /// Reason that no other identity is present.
    pub other_identities_absence_reason: Option<String>,
    /// Ready-to-display lifecycle state.
    pub lifecycle: String,
    /// Age of the newest observation.
    pub newest_observation_age: String,
    /// Fields in display order.
    pub fields: Vec<DisplayTrafficDetailField>,
}

impl From<pilotage_presentation::LayerSourceState> for DisplayLayerSourceState {
    fn from(value: pilotage_presentation::LayerSourceState) -> Self {
        match value {
            pilotage_presentation::LayerSourceState::Live => Self::Live,
            pilotage_presentation::LayerSourceState::Stale => Self::Stale,
            pilotage_presentation::LayerSourceState::Absent => Self::Absent,
        }
    }
}

impl From<pilotage_presentation::LayerControl> for DisplayLayerControl {
    fn from(value: pilotage_presentation::LayerControl) -> Self {
        Self {
            id: value.id,
            title: value.title,
            enabled: value.enabled,
            source_state: value.source_state.into(),
            source_state_label: value.source_state_label,
            source_detail: value.source_detail,
        }
    }
}

impl From<PresentationRadioBand> for pilotage_presentation::RadioBand {
    fn from(value: PresentationRadioBand) -> Self {
        match value {
            PresentationRadioBand::Adsb1090 => Self::Adsb1090,
            PresentationRadioBand::Uat978 => Self::Uat978,
        }
    }
}

impl From<PresentationRadioState> for pilotage_presentation::RadioReceptionState {
    fn from(value: PresentationRadioState) -> Self {
        match value {
            PresentationRadioState::Checking => Self::Checking,
            PresentationRadioState::PermissionDenied => Self::PermissionDenied,
            PresentationRadioState::DriverDisabled => Self::DriverDisabled,
            PresentationRadioState::Unplugged => Self::Unplugged,
            PresentationRadioState::Ready => Self::Ready,
            PresentationRadioState::Streaming => Self::Streaming,
            PresentationRadioState::Suspended => Self::Suspended,
            PresentationRadioState::Underpowered => Self::Underpowered,
            PresentationRadioState::EnumerationFailure => Self::EnumerationFailure,
            PresentationRadioState::EndpointFailure => Self::EndpointFailure,
            PresentationRadioState::DeviceRemoved => Self::DeviceRemoved,
        }
    }
}

impl From<PresentationReceiverObservation> for pilotage_presentation::RadioReceiverObservation {
    fn from(value: PresentationReceiverObservation) -> Self {
        Self {
            band: value.band.into(),
            state: value.state.into(),
        }
    }
}

impl From<pilotage_presentation::TrafficListItem> for DisplayTrafficListItem {
    fn from(value: pilotage_presentation::TrafficListItem) -> Self {
        Self {
            id: value.id,
            title: value.title,
            summary: value.summary,
        }
    }
}

impl From<pilotage_presentation::TrafficDetailField> for DisplayTrafficDetailField {
    fn from(value: pilotage_presentation::TrafficDetailField) -> Self {
        Self {
            id: value.id,
            title: value.title,
            value: value.value,
            age: value.age,
            source: value.source,
            absence_reason: value.absence_reason,
        }
    }
}

impl From<pilotage_presentation::TrafficDetail> for DisplayTrafficDetail {
    fn from(value: pilotage_presentation::TrafficDetail) -> Self {
        Self {
            id: value.id,
            title: value.title,
            primary_identity: value.primary_identity,
            other_identities: value.other_identities,
            other_identities_absence_reason: value.other_identities_absence_reason,
            lifecycle: value.lifecycle,
            newest_observation_age: value.newest_observation_age,
            fields: value.fields.into_iter().map(Into::into).collect(),
        }
    }
}
