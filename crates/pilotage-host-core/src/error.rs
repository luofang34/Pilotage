//! Why a core call did not complete.

/// A refused core call.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HostCoreError {
    /// The call is part of the published interface and this build does
    /// not carry its implementation yet.
    #[error("{operation} is not available in this build of the host core")]
    NotReady {
        /// The call that is not available.
        operation: &'static str,
    },
    /// The time given goes back from the time of an earlier call.
    #[error("time went back: {now_ns} ns after {previous_ns} ns")]
    TimeWentBack {
        /// The time of this call, nanoseconds since the core's origin.
        now_ns: u64,
        /// The time of the earlier call.
        previous_ns: u64,
    },
    /// The mission did not build from the loaded plan.
    #[error("the mission did not build: {source}")]
    MissionBuild {
        /// The engine's refusal.
        #[source]
        source: Box<pilotage_mission::MissionBuildError>,
    },
    /// The adapter did not build.
    #[error("the Aviate adapter did not build: {source}")]
    Adapter {
        /// The adapter's refusal.
        #[source]
        source: Box<pilotage_adapter_aviate::AviateAdapterError>,
    },
}
