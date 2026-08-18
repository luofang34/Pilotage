//! The camera link's build-variant seam: a simulation build holds a
//! live sidecar client, a flight build holds an uninhabited type so the
//! camera field is structurally `None`.

/// The live sidecar client handle in a simulation build; a flight build
/// has no video producer, so the type is uninhabited and the camera
/// field is structurally `None`.
#[cfg(feature = "sim")]
pub(super) type CameraBridge = pilotage_sim_video::BridgeClient;
#[cfg(not(feature = "sim"))]
pub(super) type CameraBridge = std::convert::Infallible;

/// The no-video triple of a flight build.
#[cfg(not(feature = "sim"))]
pub(super) fn no_camera() -> (
    Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>>,
    Option<CameraBridge>,
    Option<tokio::task::JoinHandle<()>>,
) {
    (None, None, None)
}
