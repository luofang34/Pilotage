//! Traffic list and detail display values.

use surveillance_core::{
    AddressNamespace, AirGroundState, AirspeedKind, Band, DeliveryPath, EmergencyState,
    FieldProvenance, HeadingReference, ObservationOrigin, TimedField, TrackKey, TrackPhase,
    TrackSnapshot, TrackSnapshotHandle, VelocityObservation, VerticalRateSource, Wgs84Position,
};

/// One traffic item that has no map position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrafficListItem {
    /// Stable display identity.
    pub id: String,
    /// Primary display text.
    pub title: String,
    /// Ready-to-display lifecycle summary.
    pub summary: String,
}

/// One traffic field and its evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrafficDetailField {
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrafficDetail {
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
    pub fields: Vec<TrafficDetailField>,
}

pub(crate) fn detail_for(handle: &TrackSnapshotHandle, now_micros: u64) -> TrafficDetail {
    let track = handle.snapshot();
    let other_identities: Vec<_> = track
        .identities
        .associated()
        .iter()
        .copied()
        .map(identity_label)
        .collect();
    let other_identities_absence_reason = other_identities
        .is_empty()
        .then(|| "No other identity is associated with this track.".into());
    TrafficDetail {
        id: crate::traffic::traffic_id(handle.producer_instance_id().get(), track.id.get()),
        title: traffic_title(track),
        primary_identity: identity_label(track.key),
        other_identities,
        other_identities_absence_reason,
        lifecycle: phase_label(track.phase).into(),
        newest_observation_age: format_age(
            now_micros.saturating_sub(track.last_observed_at_micros),
        ),
        fields: detail_fields(track, now_micros),
    }
}

pub(crate) fn positionless_item(
    handle: &TrackSnapshotHandle,
    now_micros: u64,
) -> Option<TrafficListItem> {
    let track = handle.snapshot();
    if track.position.is_some() {
        return None;
    }
    Some(TrafficListItem {
        id: crate::traffic::traffic_id(handle.producer_instance_id().get(), track.id.get()),
        title: traffic_title(track),
        summary: format!(
            "Position absent · {} · Newest observation {}",
            phase_label(track.phase),
            format_age(now_micros.saturating_sub(track.last_observed_at_micros))
        ),
    })
}

fn detail_fields(track: &TrackSnapshot, now_micros: u64) -> Vec<TrafficDetailField> {
    vec![
        field(
            "position",
            "Position",
            track.position.as_ref(),
            now_micros,
            format_position,
            "The track has no position observation.",
        ),
        field(
            "pressure-altitude",
            "Pressure altitude",
            track.pressure_altitude_ft.as_ref(),
            now_micros,
            |value| Some(format!("{value} ft")),
            "The track has no pressure altitude observation.",
        ),
        field(
            "geometric-altitude",
            "Geometric altitude",
            track.geometric_altitude_ft.as_ref(),
            now_micros,
            |value| Some(format!("{value} ft")),
            "The track has no geometric altitude observation.",
        ),
        field(
            "velocity",
            "Velocity",
            track.velocity.as_ref(),
            now_micros,
            format_velocity,
            "The track has no velocity observation.",
        ),
        field(
            "callsign",
            "Callsign",
            track.callsign.as_ref(),
            now_micros,
            |value| nonempty(value.text.trim()),
            "The track has no callsign observation.",
        ),
        field(
            "transponder-code",
            "Transponder code",
            track.squawk.as_ref(),
            now_micros,
            |value| Some(format!("{:04}", value.code())),
            "The track has no transponder code observation.",
        ),
        field(
            "air-ground",
            "Air-ground state",
            track.air_ground.as_ref(),
            now_micros,
            |value| Some(air_ground_label(*value).into()),
            "The track has no air-ground observation.",
        ),
        field(
            "emergency",
            "Emergency state",
            track.emergency.as_ref(),
            now_micros,
            |value| Some(emergency_label(*value).into()),
            "The track has no emergency observation.",
        ),
    ]
}

fn field<T>(
    id: &'static str,
    title: &'static str,
    timed: Option<&TimedField<T>>,
    now_micros: u64,
    format_value: impl FnOnce(&T) -> Option<String>,
    absence_reason: &'static str,
) -> TrafficDetailField {
    let Some(timed) = timed else {
        return absent_field(id, title, absence_reason);
    };
    let Some(value) = format_value(&timed.value) else {
        return absent_field(id, title, absence_reason);
    };
    TrafficDetailField {
        id: id.into(),
        title: title.into(),
        value: Some(value),
        age: Some(field_age(timed, now_micros)),
        source: Some(source_label(timed.provenance)),
        absence_reason: None,
    }
}

fn absent_field(
    id: &'static str,
    title: &'static str,
    absence_reason: &'static str,
) -> TrafficDetailField {
    TrafficDetailField {
        id: id.into(),
        title: title.into(),
        value: None,
        age: None,
        source: None,
        absence_reason: Some(absence_reason.into()),
    }
}

fn traffic_title(track: &TrackSnapshot) -> String {
    track
        .callsign
        .as_ref()
        .and_then(|field| nonempty(field.value.text.trim()))
        .unwrap_or_else(|| identity_label(track.key))
}

fn identity_label(key: TrackKey) -> String {
    let namespace = match key.namespace {
        AddressNamespace::Icao => "ICAO",
        AddressNamespace::AdsbNonIcao => "ADS-B non-ICAO",
        AddressNamespace::SelfAssigned => "Self-assigned",
        AddressNamespace::TisbTrackFile => "TIS-B track file",
        AddressNamespace::TisbModeATrackFile => "TIS-B Mode A track file",
        AddressNamespace::SurfaceVehicle => "Surface vehicle",
        AddressNamespace::FixedBeacon => "Fixed beacon",
        _ => "Unknown namespace",
    };
    format!("{namespace} {:06X}", key.address)
}

fn format_position(value: &Wgs84Position) -> Option<String> {
    Some(format!(
        "{:.5}°, {:.5}°",
        value.latitude_deg, value.longitude_deg
    ))
}

fn format_velocity(value: &VelocityObservation) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(speed) = value.ground_speed_kt {
        parts.push(format!("{speed:.0} kt ground speed"));
    }
    if let Some(track) = value.track_angle_deg_true {
        parts.push(format!("{track:.0}° true track"));
    }
    if let Some(speed) = value.airspeed_kt {
        let kind = value.airspeed_kind.map_or("airspeed", airspeed_label);
        parts.push(format!("{speed:.0} kt {kind}"));
    }
    if let Some(heading) = value.heading_deg {
        let reference = value.heading_reference.map_or("heading", heading_label);
        parts.push(format!("{heading:.0}° {reference}"));
    }
    if let Some(rate) = value.vertical_rate_fpm {
        let source = value
            .vertical_rate_source
            .map_or("vertical rate", vertical_rate_label);
        parts.push(format!("{rate:+} ft/min {source}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn source_label(provenance: FieldProvenance) -> String {
    let band = provenance.origin.band().map_or("No radio band", band_label);
    let delivery = delivery_label(provenance.source.delivery);
    let origin = origin_label(provenance.origin);
    if provenance.source.is_unspecified() {
        format!("{band} · {delivery} · {origin}")
    } else {
        format!(
            "{band} · {delivery} · {origin} · input {}, epoch {}",
            provenance.source.id, provenance.source.epoch
        )
    }
}

fn field_age<T>(field: &TimedField<T>, now_micros: u64) -> String {
    let age = format_age(field.age_micros(now_micros));
    if field.time.is_established() {
        age
    } else {
        format!("At least {age}")
    }
}

fn format_age(age_micros: u64) -> String {
    if age_micros < 1_000_000 {
        return format!("{} ms old", age_micros / 1_000);
    }
    if age_micros < 60_000_000 {
        return format!("{:.1} s old", age_micros as f64 / 1_000_000.0);
    }
    format!("{:.1} min old", age_micros as f64 / 60_000_000.0)
}

fn phase_label(phase: TrackPhase) -> &'static str {
    match phase {
        TrackPhase::Active => "Active",
        TrackPhase::Coasting => "Coasting",
        _ => "Unknown",
    }
}

fn band_label(band: Band) -> &'static str {
    match band {
        Band::Adsb1090 => "1090 MHz",
        Band::Uat978 => "978 MHz",
        _ => "Unknown radio band",
    }
}

fn delivery_label(delivery: DeliveryPath) -> &'static str {
    match delivery {
        DeliveryPath::Unspecified => "Unspecified link",
        DeliveryPath::LocalReceiver => "Local receiver",
        DeliveryPath::InstalledAvionics => "Installed avionics",
        DeliveryPath::NetworkProvider => "Network provider",
        DeliveryPath::Replay => "Replay link",
        DeliveryPath::Simulator => "Simulator link",
        _ => "Unknown link",
    }
}

fn origin_label(origin: ObservationOrigin) -> &'static str {
    match origin {
        ObservationOrigin::AdsbDirect { .. } => "Direct ADS-B",
        ObservationOrigin::ModeSReply => "Mode S reply",
        ObservationOrigin::AdsR { .. } => "ADS-R",
        ObservationOrigin::Tisb { .. } => "TIS-B",
        ObservationOrigin::Radar => "Radar",
        ObservationOrigin::Mlat => "Multilateration",
        ObservationOrigin::ProviderFused => "Provider fusion",
        ObservationOrigin::PanelFused => "Panel fusion",
        ObservationOrigin::Replay => "Replay origin",
        ObservationOrigin::Unknown => "Unknown origin",
        _ => "Unknown origin",
    }
}

fn air_ground_label(state: AirGroundState) -> &'static str {
    match state {
        AirGroundState::Subsonic | AirGroundState::Supersonic => "Airborne",
        AirGroundState::Ground => "On ground",
        AirGroundState::Reserved => "Reserved",
        _ => "Unknown",
    }
}

fn emergency_label(state: EmergencyState) -> &'static str {
    match state {
        EmergencyState::None => "None",
        EmergencyState::General => "General emergency",
        EmergencyState::Medical => "Medical emergency",
        EmergencyState::MinimumFuel => "Minimum fuel",
        EmergencyState::NoCommunication => "No radio communication",
        EmergencyState::UnlawfulInterference => "Unlawful interference",
        EmergencyState::DownedAircraft => "Downed aircraft",
        EmergencyState::Reserved => "Reserved",
        _ => "Unknown",
    }
}

fn airspeed_label(kind: AirspeedKind) -> &'static str {
    match kind {
        AirspeedKind::Indicated => "indicated airspeed",
        AirspeedKind::True => "true airspeed",
        _ => "airspeed",
    }
}

fn heading_label(reference: HeadingReference) -> &'static str {
    match reference {
        HeadingReference::TrueNorth => "true heading",
        HeadingReference::MagneticNorth => "magnetic heading",
        _ => "heading",
    }
}

fn vertical_rate_label(source: VerticalRateSource) -> &'static str {
    match source {
        VerticalRateSource::Barometric => "barometric",
        VerticalRateSource::Geometric => "geometric",
        _ => "vertical rate",
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}
