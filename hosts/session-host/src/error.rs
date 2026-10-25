//! Typed errors for the session-host binary (ADR-0015: no `anyhow` in
//! library or binary code, typed `thiserror` enums throughout).

use crate::cli::CliError;

/// Failures that can prevent the session host from starting or running.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// Bind or admission configuration is invalid.
    #[error(transparent)]
    Network(#[from] crate::runtime::NetworkError),
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
    /// A present Aviate telemetry endpoint was not a socket address.
    #[error("invalid {variable} value {value:?}: {source}")]
    AviateEndpoint {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The rejected value.
        value: String,
        /// The socket-address parse failure.
        #[source]
        source: std::net::AddrParseError,
    },
    /// An Aviate telemetry endpoint could not be decoded as UTF-8.
    #[error("{variable} is not valid UTF-8: {source}")]
    AviateEndpointEncoding {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The environment decoding failure.
        #[source]
        source: std::env::VarError,
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
    /// A present Aviate setting was outside the values the adapter accepts.
    #[error("invalid {variable} value {value:?} (expected {expected})")]
    AviateSetting {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The rejected value.
        value: String,
        /// What the variable accepts.
        expected: &'static str,
    },
    /// A present PX4 setting was outside the values the adapter accepts.
    #[error("invalid {variable} value {value:?} (expected {expected})")]
    Px4Setting {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The rejected value.
        value: String,
        /// What the variable accepts.
        expected: &'static str,
    },
    /// A PX4 endpoint or setting could not be decoded as UTF-8.
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
    /// `PILOTAGE_SIM_CAMERA_PORT` is not a port a producer can dial.
    #[error("PILOTAGE_SIM_CAMERA_PORT {value:?} is not a port from 1 to 65535")]
    SimCameraPort {
        /// The refused value.
        value: String,
    },
    /// The simulator camera port could not be bound on loopback.
    #[error("failed to bind the sim camera port 127.0.0.1:{port}: {source}")]
    SimCameraBind {
        /// The configured port.
        port: u16,
        /// The bind failure.
        #[source]
        source: std::io::Error,
    },
    /// The adapter already produces video, and a second producer would
    /// claim the same camera sources.
    #[error(
        "the {adapter} adapter already produces video; turn its camera off to attach the sim camera"
    )]
    SimCameraConflict {
        /// The adapter that produces video.
        adapter: &'static str,
    },
    /// The simulator camera was requested from a build without the sim
    /// feature.
    #[error("the sim camera is not in this build (built without the sim feature)")]
    SimCameraNotInBuild,
    /// Starting the Aviate MAVLink telemetry link failed.
    #[error("failed to start the Aviate adapter: {0}")]
    AviateAdapter(#[source] pilotage_adapter_aviate::AviateAdapterError),
    /// Starting the PX4 MAVLink link failed.
    #[error("failed to start the PX4 adapter: {0}")]
    Px4Adapter(#[source] pilotage_adapter_px4::Px4AdapterError),
    /// The ArduPilot adapter requires an explicit simulation profile.
    #[error("PILOTAGE_ARDUPILOT_PROFILE is required and must be set to simulation")]
    ArduPilotProfileMissing,
    /// A present ArduPilot profile did not name the only supported policy.
    #[error("invalid PILOTAGE_ARDUPILOT_PROFILE value {value:?} (expected simulation)")]
    ArduPilotProfile {
        /// The rejected value.
        value: String,
    },
    /// The ArduPilot endpoint was not a socket address.
    #[error("invalid PILOTAGE_ARDUPILOT_ADDR value {value:?}: {source}")]
    ArduPilotEndpoint {
        /// The rejected value.
        value: String,
        /// The socket-address parse failure.
        #[source]
        source: std::net::AddrParseError,
    },
    /// An ArduPilot variable could not be decoded as UTF-8.
    #[error("{variable} is not valid UTF-8: {source}")]
    ArduPilotEnvEncoding {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The environment decoding failure.
        #[source]
        source: std::env::VarError,
    },
    /// Starting the ArduPilot MAVLink link failed.
    #[error("failed to start the ArduPilot adapter: {0}")]
    ArduPilotAdapter(#[source] pilotage_adapter_ardupilot::ArduPilotAdapterError),
    /// The Betaflight adapter's environment configuration is invalid.
    #[error(transparent)]
    BetaflightConfig(#[from] BetaflightConfigError),
    /// The DJI bench transport or its explicit configuration failed.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[error("DJI adapter: {0}")]
    Dji(#[from] crate::runtime::dji::DjiHostError),
    /// The simulated multicopter's environment configuration is invalid.
    #[cfg(feature = "sim")]
    #[error("simulated multicopter: {0}")]
    SimMulticopter(#[from] crate::runtime::sim_multicopter::SimMulticopterHostError),
    /// A host environment variable could not be decoded as UTF-8.
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
    /// `PILOTAGE_MISSION_CRUISE_SPEED` was not a number.
    #[error("invalid PILOTAGE_MISSION_CRUISE_SPEED value {value:?}: {source}")]
    MissionCruiseSpeed {
        /// The rejected value.
        value: String,
        /// The float parse failure.
        #[source]
        source: std::num::ParseFloatError,
    },
    /// `PILOTAGE_MISSION_CRUISE_SPEED` parsed but cannot be flown: the speed
    /// must be finite and positive.
    #[error("PILOTAGE_MISSION_CRUISE_SPEED must be a finite speed > 0 m/s, got {speed}")]
    MissionCruiseSpeedRange {
        /// The rejected speed in meters per second.
        speed: f64,
    },
    /// `PILOTAGE_MISSION_OWNSHIP` named no known ownship source.
    #[error(
        "invalid PILOTAGE_MISSION_OWNSHIP value {value:?} (expected \"simulation-truth\" or \"vehicle-estimate\")"
    )]
    MissionOwnshipSource {
        /// The rejected value.
        value: String,
    },
    /// `PILOTAGE_MISSION_TARGET` named no known execution target.
    #[error(
        "invalid PILOTAGE_MISSION_TARGET value {value:?} (expected \"simulator\" or \"real-vehicle\")"
    )]
    MissionTarget {
        /// The rejected value.
        value: String,
    },
    /// A real vehicle exports no simulation truth, so that pairing would
    /// wait for a first solution for the whole mission.
    #[error(
        "PILOTAGE_MISSION_TARGET=real-vehicle requires PILOTAGE_MISSION_OWNSHIP=vehicle-estimate"
    )]
    MissionTruthOnRealVehicle,
    /// Both a route and an area were given; a mission flies one plan.
    #[error("set one of PILOTAGE_MISSION_ROUTE and PILOTAGE_MISSION_AREA, not both")]
    MissionPlanConflict,
    /// `PILOTAGE_MISSION_AREA` was not at least three `lat_deg,lon_deg`
    /// pairs separated by semicolons.
    #[error(
        "invalid PILOTAGE_MISSION_AREA value {value:?} (expected at least three \"lat_deg,lon_deg\" pairs separated by ';')"
    )]
    MissionArea {
        /// The rejected value.
        value: String,
    },
    /// The coordinator's member list cannot be read.
    #[error("cannot read the coordinator members {path:?}: {reason}")]
    CoordinatorMembers {
        /// The member file.
        path: std::path::PathBuf,
        /// What is wrong.
        reason: String,
    },
    /// The coordinator's handover journal cannot be read: the coordinator
    /// does not start, as it cannot show which work may be unflown.
    #[error("cannot read the coordinator handover journal {path:?}: {reason}")]
    CoordinatorJournal {
        /// The journal file.
        path: std::path::PathBuf,
        /// What is wrong.
        reason: String,
    },
    /// The lane journal cannot be read, decoded or restored: the host does
    /// not start, as it cannot show what the run in progress did.
    #[error("cannot restore the mission lane from its journal {path:?}: {reason}")]
    LaneJournal {
        /// The journal file.
        path: std::path::PathBuf,
        /// What is wrong.
        reason: String,
    },
    /// A mission lane variable did not parse or is outside its range.
    #[error("invalid mission lane setting {variable}={value:?}")]
    LaneConfig {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The rejected value, or why it was rejected.
        value: String,
    },
    /// A survey parameter variable did not parse or is outside its range.
    #[error("invalid {variable} value {value:?}: {reason}")]
    MissionSurveyParameter {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The rejected value.
        value: String,
        /// What the variable accepts.
        reason: &'static str,
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

/// Why the Betaflight adapter's environment configuration was refused.
/// Parsing fails closed: a present but malformed value never falls back
/// to a default.
#[derive(Debug, thiserror::Error)]
pub enum BetaflightConfigError {
    /// `PILOTAGE_BETAFLIGHT_PROFILE` is unset; a direct start must name it.
    #[error("PILOTAGE_BETAFLIGHT_PROFILE is not set (the one supported profile is \"simulation\")")]
    ProfileMissing,
    /// The profile is not one this adapter implements.
    #[error(
        "unsupported PILOTAGE_BETAFLIGHT_PROFILE {value:?} (the one supported profile is \"simulation\")"
    )]
    Profile {
        /// The rejected value.
        value: String,
    },
    /// A Betaflight variable could not be decoded as UTF-8.
    #[error("{variable} is not valid UTF-8: {source}")]
    Encoding {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The environment decoding failure.
        #[source]
        source: std::env::VarError,
    },
    /// The MSP endpoint was not a socket address.
    #[error("invalid {variable} value {value:?}: {source}")]
    Endpoint {
        /// The environment variable being parsed.
        variable: &'static str,
        /// The rejected value.
        value: String,
        /// The socket-address parse failure.
        #[source]
        source: std::net::AddrParseError,
    },
}
