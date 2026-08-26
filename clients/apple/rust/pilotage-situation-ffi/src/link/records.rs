//! FFI records and events for the host link.

use pilotage_client_session::Admission;
use pilotage_protocol::wire;

use crate::error::FfiError;

/// How to reach one session host.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LinkConfig {
    /// WebTransport URL, `https://host:port/pilotage`.
    pub url: String,
    /// SHA-256 of the host's certificate, hex. Empty accepts any
    /// certificate and is for loopback development only.
    pub certificate_sha256_hex: String,
    /// Name the host records for this connection.
    pub client_name: String,
}

/// One advertised intent capability.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LinkIntentCapability {
    /// Wire `IntentFamily` value.
    pub family: i32,
    /// Horizontal linear bound (m/s or m).
    pub max_linear: f32,
    /// Vertical linear bound; zero falls back to `max_linear`.
    pub max_vertical: f32,
    /// Angular bound (rad/s or rad).
    pub max_angular: f32,
}

/// One advertised control scope.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LinkScope {
    /// Scope identity string.
    pub scope: String,
    /// Typed intent families the scope accepts.
    pub intents: Vec<LinkIntentCapability>,
    /// Typed discrete action codes the scope accepts (the wire
    /// `ControlAction` values), so the shell can show a control
    /// exactly when the host offers its action.
    pub actions: Vec<i32>,
}

/// One offered vehicle.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LinkVehicle {
    /// Vehicle identity.
    pub vehicle_id: u64,
    /// Host display name.
    pub display_name: String,
    /// Published control scopes.
    pub scopes: Vec<LinkScope>,
}

/// Identity of the active control-feel artifact.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LinkControlFeelIdentity {
    /// Stable profile identity.
    pub profile_id: String,
    /// Selected control-feel mode.
    pub mode: LinkControlFeelMode,
    /// Profile schema version.
    pub schema_version: u32,
    /// SHA-256 of the complete profile artifact.
    pub profile_sha256: Vec<u8>,
    /// SHA-256 of the bound input-device profile.
    pub device_profile_sha256: Vec<u8>,
    /// SHA-256 of the bound flight-controller artifact.
    pub flight_controller_sha256: Vec<u8>,
}

/// One control-feel mode from the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum LinkControlFeelMode {
    /// The host sent no supported mode.
    Unspecified,
    /// The response gives priority to exact small inputs.
    Precision,
    /// The response balances small inputs and full inputs.
    Balanced,
    /// The response gives priority to fast full inputs.
    Agile,
    /// The response keeps the established command behavior.
    LegacyCompatibility,
}

/// What an admitted session offers, in the shell's vocabulary.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LinkCatalog {
    /// The session the host assigned.
    pub session_id: u64,
    /// This connection's principal identity.
    pub principal_id: u64,
    /// The host's version string.
    pub host_version: String,
    /// Offered vehicles.
    pub vehicles: Vec<LinkVehicle>,
    /// Active control-feel identity, when the host has one.
    pub control_feel: Option<LinkControlFeelIdentity>,
}

/// One typed link event for the shell.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum LinkEvent {
    /// The host admitted this session.
    Admitted {
        /// What the session offers.
        catalog: LinkCatalog,
    },
    /// The lease outcome for a requested scope.
    LeaseChanged {
        /// Whether control is held now.
        held: bool,
        /// The scope the outcome describes.
        scope: String,
        /// Denial or release detail, empty on a grant.
        detail: String,
    },
    /// The arm telegraph moved: the operator's order, the flight
    /// controller's own answer, and where the two stand. Phase 0 is in
    /// sync, 1 awaiting the FC's answer, 2 refused (detail says why),
    /// 3 dropped — the vehicle left the ordered state on its own.
    ArmTelegraph {
        /// Whether the lever orders the vehicle live.
        ordered_armed: bool,
        /// The FC's report: 0 unknown, 1 disarmed, 2 armed.
        confirmed: u32,
        /// The reconciliation phase code.
        phase: u32,
        /// The refusal reason, when phase is 2.
        detail: String,
    },
    /// A pad was selected and resolved against the profile registry;
    /// the hints name the arm and disarm controls in the operator's
    /// terms, from profile data.
    PadSelected {
        /// The resolved device profile's label.
        label: String,
        /// The operator-facing name of the arm control.
        arm_hint: String,
        /// The operator-facing name of the disarm control.
        disarm_hint: String,
    },
    /// An arm or disarm press fired while control output was gated; the
    /// press is consumed, so the operator is owed the explanation.
    PressSuppressed {
        /// 1 arm, 2 disarm.
        action: i32,
    },
    /// The gimbal quasimode started or stopped capturing the stick.
    GimbalCapture {
        /// Whether the right stick is captured for the gimbal now.
        active: bool,
    },
    /// A link-side observation worth the operator's eye, in words.
    Notice {
        /// The observation.
        text: String,
    },
    /// The host rejected a control frame; fencing feedback.
    ControlRejected {
        /// The rejected frame's sequence.
        sequence: u32,
        /// The host's typed rejection reason, as the wire enum's number.
        reason: i32,
    },
    /// The transport is down.
    Down {
        /// Next scheduled attempt on the link's monotonic clock,
        /// absent when retry has stopped.
        retry_at_ms: Option<u64>,
    },
    /// The link stopped for good.
    Stopped {
        /// The typed fault, formatted.
        reason: String,
    },
    /// Another principal asked for the scope this client holds. The
    /// operator decides; nothing changes hands until they do.
    TakeoverAsked {
        /// The principal asking.
        from_principal: u64,
        /// The scope asked for.
        scope: String,
    },
    /// One discrete action's outcome, correlated to the press.
    ActionResult {
        /// The wire `ControlAction` code the result answers.
        action: i32,
        /// Whether the vehicle executed it.
        accepted: bool,
        /// Adapter-supplied reason when not accepted; empty on acceptance.
        detail: String,
    },
    /// Where the vehicle says it is, from the telemetry lane.
    ///
    /// The situation session ingests surveillance, weather and terrain; a
    /// vehicle under this operator's own control is on none of those, so its
    /// position reaches the map through here or not at all.
    ///
    /// One lane supplies the whole of it. A position from the simulator's
    /// oracle beside a heading from the estimator would draw one measurement
    /// turned by another, and nothing on the mark could say so.
    VehicleFix {
        /// Latitude in degrees.
        latitude_deg: f64,
        /// Longitude in degrees.
        longitude_deg: f64,
        /// Where the nose points, true, when the lane states an attitude.
        heading_deg: Option<f64>,
        /// Track over the ground, when the lane states a velocity above the
        /// floor below which a direction is noise rather than a course.
        course_deg: Option<f64>,
        /// Speed over the ground, m/s, alongside `course_deg`.
        ground_speed_mps: Option<f64>,
        /// Whether the simulator's oracle supplied this, rather than the
        /// vehicle's own estimator.
        from_simulator: bool,
        /// Whether the position is a NEW measurement rather than a repeat.
        ///
        /// A mark goes stale on the age of the last position MEASURED, not
        /// the last one delivered: a host relaying a frozen block delivers
        /// forever, and a mark timed from delivery would never go stale.
        fix_advanced: bool,
    },
    /// One second of link accounting. What blinks on screen shows up
    /// here as a number: telemetry that arrives in bursts, frames the
    /// host rejected, actions that never got a result.
    Stats {
        /// Telemetry samples ingested this second.
        telemetry_per_second: u32,
        /// State frames delivered to the shell this second.
        state_frames_per_second: u32,
        /// Control frames sent this second.
        control_frames_per_second: u32,
        /// Control frames the host rejected this second.
        rejected_per_second: u32,
        /// Discrete action results seen this second, `granted` counting
        /// accepted ones.
        action_results_per_second: u32,
        /// Bytes parked in stream reassembly buffers right now. A
        /// figure that keeps climbing is a desynchronized stream.
        stream_pending_bytes: u64,
        /// Video frames received this second, across every source.
        video_frames_per_second: u32,
        /// Encoded video bytes received this second.
        video_bytes_per_second: u64,
    },
}

impl LinkCatalog {
    pub(crate) fn from_admission(admission: &Admission) -> Self {
        Self {
            session_id: admission.session_id,
            principal_id: admission.principal_id,
            host_version: admission.host_version.clone(),
            control_feel: admission
                .control_feel
                .as_ref()
                .map(|identity| LinkControlFeelIdentity {
                    profile_id: identity.profile_id.clone(),
                    mode: control_feel_mode(identity.mode),
                    schema_version: identity.schema_version,
                    profile_sha256: identity.profile_sha256.clone(),
                    device_profile_sha256: identity.device_profile_sha256.clone(),
                    flight_controller_sha256: identity.flight_controller_sha256.clone(),
                }),
            vehicles: admission
                .vehicles
                .iter()
                .map(|vehicle| LinkVehicle {
                    vehicle_id: vehicle.vehicle_id,
                    display_name: vehicle.display_name.clone(),
                    scopes: vehicle
                        .scopes
                        .iter()
                        .map(|scope| LinkScope {
                            scope: scope.scope.clone(),
                            intents: scope
                                .intents
                                .iter()
                                .map(|intent| LinkIntentCapability {
                                    family: intent.family,
                                    max_linear: intent.max_linear,
                                    max_vertical: intent.max_vertical,
                                    max_angular: intent.max_angular,
                                })
                                .collect(),
                            actions: scope.actions.iter().map(|action| action.action).collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

fn control_feel_mode(value: i32) -> LinkControlFeelMode {
    match wire::ControlFeelMode::try_from(value) {
        Ok(wire::ControlFeelMode::Precision) => LinkControlFeelMode::Precision,
        Ok(wire::ControlFeelMode::Balanced) => LinkControlFeelMode::Balanced,
        Ok(wire::ControlFeelMode::Agile) => LinkControlFeelMode::Agile,
        Ok(wire::ControlFeelMode::LegacyCompatibility) => LinkControlFeelMode::LegacyCompatibility,
        Ok(wire::ControlFeelMode::Unspecified) | Err(_) => LinkControlFeelMode::Unspecified,
    }
}

/// Parses the pinned certificate hash, or `None` for the empty
/// development value.
pub(crate) fn parse_certificate_hash(hex: &str) -> Result<Option<[u8; 32]>, FfiError> {
    if hex.is_empty() {
        return Ok(None);
    }
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(FfiError::HostLink {
            message: "certificate hash must be 64 hex digits".to_owned(),
        });
    }
    let mut digest = [0_u8; 32];
    for (index, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = char::from(chunk[0]).to_digit(16).unwrap_or(0);
        let low = char::from(chunk[1]).to_digit(16).unwrap_or(0);
        digest[index] = u8::try_from((high << 4) | low).unwrap_or(0);
    }
    Ok(Some(digest))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::parse_certificate_hash;

    #[test]
    fn the_development_value_pins_nothing_and_says_so() {
        assert_eq!(parse_certificate_hash("").expect("empty is allowed"), None);
    }

    #[test]
    fn a_pinned_hash_round_trips_and_a_malformed_one_is_refused() {
        let hex = "ab".repeat(32);
        let digest = parse_certificate_hash(&hex)
            .expect("64 hex digits parse")
            .expect("a digest is pinned");
        assert_eq!(digest, [0xab_u8; 32]);
        assert!(
            parse_certificate_hash("abcd").is_err(),
            "short input is refused"
        );
        assert!(
            parse_certificate_hash(&"zz".repeat(32)).is_err(),
            "non-hex input is refused"
        );
    }
}
