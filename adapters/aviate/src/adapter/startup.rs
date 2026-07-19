//! Adapter construction: binding the profile's source roles, the flight
//! uplink, and the camera bridge into a ready [`AviateAdapter`].

use pilotage_protocol::VehicleId;

use super::{AviateAdapter, AviateProfile, camera, sources::bind_sources};
use crate::error::AviateAdapterError;
use crate::incarnation::{IncarnationProvider, OsIncarnationProvider};
use crate::link::LinkConfig;
use crate::uplink::FlightUplink;

impl AviateAdapter {
    /// Binds the profile's source roles and returns a ready adapter.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when a link the profile requires
    /// cannot be established (`Simulation` tolerates only a missing
    /// truth oracle).
    pub async fn start(
        vehicle: VehicleId,
        profile: AviateProfile,
        config: LinkConfig,
    ) -> Result<Self, AviateAdapterError> {
        let mut provider = OsIncarnationProvider;
        Self::start_with_incarnation_provider(vehicle, profile, config, &mut provider).await
    }

    /// Binds the vehicle link using a caller-owned attachment identity source.
    ///
    /// Aircraft integrations use this entry point to supply a persistent boot
    /// counter or source-issued UUID instead of the simulator CSPRNG provider.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when identity creation or the selected
    /// vehicle link fails.
    pub async fn start_with_incarnation_provider<P: IncarnationProvider>(
        vehicle: VehicleId,
        profile: AviateProfile,
        config: LinkConfig,
        provider: &mut P,
    ) -> Result<Self, AviateAdapterError> {
        let arm_incarnation = provider.next_incarnation_blocking()?;
        let (estimate, truth) = bind_sources(profile, config, provider).await?;
        // Oracle-only sessions bind no uplink at all: with no motion
        // scope advertised, operational control is structurally absent
        // rather than rejected case by case. Elsewhere a failed uplink
        // bind degrades to telemetry-only rather than failing the
        // adapter: displaying a flight you cannot command beats
        // displaying nothing.
        let uplink = if profile == AviateProfile::OracleOnly {
            None
        } else {
            match FlightUplink::new() {
                Ok(mut uplink) => {
                    uplink.set_expected_source(config.system_id, config.component_id);
                    Some(uplink)
                }
                Err(error) => {
                    tracing::warn!(%error, "flight uplink unavailable; telemetry-only");
                    None
                }
            }
        };
        let (frames, camera_bridge, frame_forwarder) = camera::spawn_camera_bridge().await;
        Ok(Self {
            vehicle,
            estimate,
            truth,
            uplink,
            frames,
            _camera_bridge: camera_bridge,
            _frame_forwarder: frame_forwarder,
            arm: None,
            arm_incarnation,
            started_at: std::time::Instant::now(),
            last_reset: None,
            reset_latch: None,
            #[cfg(test)]
            reset_spawns: 0,
            fpv_mode: false,
            link_loss_policy: None,
        })
    }
}
