//! Typed errors for the session-host binary (ADR-0015: no `anyhow` in
//! library or binary code, typed `thiserror` enums throughout).

use crate::cli::CliError;

/// Failures that can prevent the session host from starting or running.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// Command-line arguments were malformed.
    #[error("invalid command-line arguments: {0}")]
    Cli(#[source] CliError),
    /// Building the self-signed TLS identity for loopback development failed.
    #[error("failed to build self-signed identity: {0}")]
    Identity(#[source] wtransport::tls::error::InvalidSan),
    /// Binding or constructing the WebTransport server endpoint failed.
    #[error("failed to construct server endpoint: {0}")]
    Endpoint(#[source] std::io::Error),
    /// Reading the bound local address back from the endpoint failed.
    #[error("failed to read local address from endpoint: {0}")]
    LocalAddr(#[source] std::io::Error),
    /// Constructing the tokio runtime failed.
    #[error("failed to build the tokio runtime: {0}")]
    Runtime(#[source] std::io::Error),
    /// `PILOTAGE_AVIATE_PROFILE` held a value that is not a known session
    /// profile. Unknown values fail startup rather than falling back: a
    /// typo in a physical deployment must never fail open into the
    /// simulation profile.
    #[error(
        "invalid PILOTAGE_AVIATE_PROFILE value {value:?} (expected physical, simulation, or oracle-only)"
    )]
    AviateProfile {
        /// The rejected value, lossily decoded for this message.
        value: String,
    },
    /// The checked Aviate control-feel artifact violates its strict schema.
    #[error("invalid checked Aviate control-feel profile: {source}")]
    AviateDefaultControlFeelInvalid {
        /// The parse or validation failure.
        #[source]
        source: pilotage_control_feel::ProfileLoadError,
    },
    /// A physical session cannot load an unqualified local artifact.
    #[error("PILOTAGE_AVIATE_CONTROL_FEEL_PROFILE cannot select {path} for a physical session")]
    AviatePhysicalControlFeelOverride {
        /// The refused local artifact path.
        path: std::path::PathBuf,
    },
    /// The Aviate control-feel artifact could not be read.
    #[error("failed to read Aviate control-feel profile {path}: {source}")]
    AviateControlFeelRead {
        /// The requested artifact path.
        path: std::path::PathBuf,
        /// The file read failure.
        #[source]
        source: std::io::Error,
    },
    /// The Aviate control-feel artifact violates its strict schema.
    #[error("invalid Aviate control-feel profile {path}: {source}")]
    AviateControlFeelInvalid {
        /// The rejected artifact path.
        path: std::path::PathBuf,
        /// The parse or validation failure.
        #[source]
        source: pilotage_control_feel::ProfileLoadError,
    },
    /// The PX4 adapter requires an explicit simulation profile.
    #[error("PILOTAGE_PX4_PROFILE is required and must be set to simulation")]
    Px4ProfileMissing,
    /// A present PX4 profile did not name the only supported policy.
    #[error("invalid PILOTAGE_PX4_PROFILE value {value:?} (expected simulation)")]
    Px4Profile {
        /// The rejected value.
        value: String,
    },
    /// The PX4 profile could not be decoded as UTF-8.
    #[error("PILOTAGE_PX4_PROFILE is not valid UTF-8: {source}")]
    Px4ProfileEncoding {
        /// The environment decoding failure.
        #[source]
        source: std::env::VarError,
    },
    /// A present PX4 endpoint was not a socket address.
    #[error("invalid {variable} value {value:?}: {source}")]
    Px4Endpoint {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The rejected value.
        value: String,
        /// The socket-address parse failure.
        #[source]
        source: std::net::AddrParseError,
    },
    /// A PX4 endpoint could not be decoded as UTF-8.
    #[error("{variable} is not valid UTF-8: {source}")]
    Px4EndpointEncoding {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The environment decoding failure.
        #[source]
        source: std::env::VarError,
    },
    /// Spawning or connecting the Gazebo sidecar bridge failed.
    #[cfg(feature = "sim")]
    #[error("failed to start the Gazebo adapter: {0}")]
    GazeboAdapter(#[source] pilotage_adapter_gazebo::GazeboAdapterError),
    /// The selected adapter is simulation-only and absent from this build.
    #[error("adapter {adapter:?} is not in this build (built without the sim feature)")]
    AdapterNotInBuild {
        /// The requested adapter name.
        adapter: &'static str,
    },
    /// Starting the Aviate MAVLink telemetry link failed.
    #[error("failed to start the Aviate adapter: {0}")]
    AviateAdapter(#[source] pilotage_adapter_aviate::AviateAdapterError),
    /// Starting the PX4 MAVLink link failed.
    #[error("failed to start the PX4 adapter: {0}")]
    Px4Adapter(#[source] pilotage_adapter_px4::Px4AdapterError),
    /// A `PILOTAGE_MISSION_*` variable could not be decoded as UTF-8.
    #[error("{variable} is not valid UTF-8: {source}")]
    MissionVarEncoding {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The environment decoding failure.
        #[source]
        source: std::env::VarError,
    },
    /// `PILOTAGE_MISSION_ANCHOR` was not `lat_deg,lon_deg,alt_m`.
    #[error("invalid PILOTAGE_MISSION_ANCHOR value {value:?} (expected \"lat_deg,lon_deg,alt_m\")")]
    MissionAnchor {
        /// The rejected value.
        value: String,
    },
    /// `PILOTAGE_MISSION_DATE` did not parse as an ISO calendar date.
    #[error("invalid PILOTAGE_MISSION_DATE value {value:?} (expected YYYY-MM-DD): {source}")]
    MissionDate {
        /// The rejected value.
        value: String,
        /// The date parse failure.
        #[source]
        source: chrono::ParseError,
    },
    /// A navdata store needs an explicit flight date to select a cycle;
    /// guessing "today" would silently change which data a flight is
    /// packed against (ADR-0030).
    #[error("PILOTAGE_MISSION_DATE is required when PILOTAGE_MISSION_NAVDATA is a store directory")]
    MissionDateMissing,
    /// `PILOTAGE_MISSION_CRUISE_HEIGHT` was not a number.
    #[error("invalid PILOTAGE_MISSION_CRUISE_HEIGHT value {value:?}: {source}")]
    MissionCruiseHeight {
        /// The rejected value.
        value: String,
        /// The float parse failure.
        #[source]
        source: std::num::ParseFloatError,
    },
    /// `PILOTAGE_MISSION_CRUISE_HEIGHT` parsed but cannot be flown: a
    /// negative height would plan every waypoint below the launch
    /// elevation, and a non-finite one disables every altitude
    /// comparison downstream.
    #[error("PILOTAGE_MISSION_CRUISE_HEIGHT must be a finite height >= 0 m, got {height}")]
    MissionCruiseHeightRange {
        /// The rejected height in meters.
        height: f64,
    },
    /// Loading or selecting the mission navdata snapshot failed.
    #[error("failed to load mission navdata: {0}")]
    MissionNavdata(#[source] crate::mission_navdata::NavdataError),
    /// The mission plan did not build from the loaded snapshot and route.
    /// Boxed: the build error carries route/cycle context and would
    /// otherwise dominate every `Result<_, HostError>` on the startup path.
    #[error("failed to build the mission plan: {0}")]
    MissionBuild(#[source] Box<pilotage_mission::MissionBuildError>),
}
