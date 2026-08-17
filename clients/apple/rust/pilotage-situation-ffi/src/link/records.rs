//! FFI records and events for the host link.

use pilotage_client_session::Admission;

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
    /// The host rejected a control frame; fencing feedback.
    ControlRejected {
        /// The rejected frame's sequence.
        sequence: u32,
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
    },
}

impl LinkCatalog {
    pub(crate) fn from_admission(admission: &Admission) -> Self {
        Self {
            session_id: admission.session_id,
            principal_id: admission.principal_id,
            host_version: admission.host_version.clone(),
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
                        })
                        .collect(),
                })
                .collect(),
        }
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
    for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
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
