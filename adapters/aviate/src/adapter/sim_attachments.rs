//! Build-variant seam for the simulation-only attachments: a simulation
//! build holds live camera-sidecar and truth-oracle handles; a flight
//! build holds uninhabited types so both fields are structurally `None`.

/// The live sidecar client handle in a simulation build.
#[cfg(feature = "sim")]
pub(super) type CameraBridge = pilotage_sim_video::BridgeClient;
/// A flight build has no video producer.
#[cfg(not(feature = "sim"))]
pub(super) type CameraBridge = std::convert::Infallible;

/// The simulation-truth oracle handle in a simulation build.
#[cfg(feature = "sim")]
pub(super) type TruthOracle = super::shm_sampling::ShmSource;
/// A flight build has no simulator to borrow truth from.
#[cfg(not(feature = "sim"))]
pub(super) type TruthOracle = std::convert::Infallible;

/// The no-video triple of a flight build.
#[cfg(not(feature = "sim"))]
pub(super) fn no_camera() -> (
    Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>>,
    Option<CameraBridge>,
    Option<tokio::task::JoinHandle<()>>,
) {
    (None, None, None)
}

impl super::AviateAdapter {
    /// The typed fault that has fail-closed the simulation-truth source,
    /// if any. A faulted source publishes no telemetry and does not
    /// re-attach. Only the shared-memory truth source carries a
    /// fail-closed fault state; the MAVLink estimate link reports `None`
    /// here.
    #[must_use]
    pub fn shm_fault(&self) -> Option<&super::AviateAdapterError> {
        #[cfg(feature = "sim")]
        {
            self.truth.as_ref().and_then(|source| source.fault())
        }
        #[cfg(not(feature = "sim"))]
        {
            None
        }
    }

    /// The truth oracle's sample for this tick, `None` in a flight build
    /// (the oracle type is uninhabited there).
    pub(super) fn take_truth_sample(&mut self) -> Option<pilotage_adapter_api::SimTruthSample> {
        #[cfg(feature = "sim")]
        {
            self.truth.as_mut().and_then(|source| source.truth_sample())
        }
        #[cfg(not(feature = "sim"))]
        {
            self.truth
                .as_ref()
                .map(|never| -> pilotage_adapter_api::SimTruthSample { match **never {} })
        }
    }

    /// The truth oracle's session tick in nanoseconds, `None` when no
    /// oracle is bound.
    pub(super) fn truth_tick_ns(&self) -> Option<u64> {
        #[cfg(feature = "sim")]
        {
            self.truth.as_ref().map(|source| source.tick())
        }
        #[cfg(not(feature = "sim"))]
        {
            self.truth.as_ref().map(|never| -> u64 { match **never {} })
        }
    }
}
