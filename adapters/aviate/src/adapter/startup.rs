//! Adapter construction: binding the profile's source roles, the flight
//! uplink, and the camera bridge into a ready [`AviateAdapter`].

use std::collections::BTreeMap;

use pilotage_control_feel::{FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_protocol::VehicleId;

#[cfg(feature = "sim")]
use super::camera;
use super::{AviateAdapter, AviateProfile, ControlFeelIdentity, sources::bind_sources};
use crate::error::AviateAdapterError;
use crate::incarnation::{IncarnationProvider, OsIncarnationProvider};
use crate::uplink::FlightUplink;
use pilotage_mavlink::link::LinkConfig;

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
        let control_feel = compatibility_control_feel()?;
        Self::start_with_control_feel(vehicle, profile, config, control_feel).await
    }

    /// Binds the profile with a validated control-feel profile.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when a required link cannot start.
    pub async fn start_with_control_feel(
        vehicle: VehicleId,
        profile: AviateProfile,
        config: LinkConfig,
        control_feel: ValidatedFlightFeelProfile,
    ) -> Result<Self, AviateAdapterError> {
        let mut provider = OsIncarnationProvider;
        Self::start_with_control_feel_and_incarnation_provider(
            vehicle,
            profile,
            config,
            control_feel,
            &mut provider,
        )
        .await
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
        let control_feel = compatibility_control_feel()?;
        Self::start_with_control_feel_and_incarnation_provider(
            vehicle,
            profile,
            config,
            control_feel,
            provider,
        )
        .await
    }

    /// Binds an explicit control-feel profile and attachment identity source.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when the profile needs an unavailable
    /// signal or when a required vehicle link cannot start.
    pub async fn start_with_control_feel_and_incarnation_provider<P: IncarnationProvider>(
        vehicle: VehicleId,
        profile: AviateProfile,
        config: LinkConfig,
        control_feel: ValidatedFlightFeelProfile,
        provider: &mut P,
    ) -> Result<Self, AviateAdapterError> {
        validate_adapter_control_feel(&control_feel)?;
        Self::start_bound(vehicle, profile, config, control_feel, provider).await
    }

    async fn start_bound<P: IncarnationProvider>(
        vehicle: VehicleId,
        profile: AviateProfile,
        config: LinkConfig,
        control_feel: ValidatedFlightFeelProfile,
        provider: &mut P,
    ) -> Result<Self, AviateAdapterError> {
        let control_feel_identity = ControlFeelIdentity::from_profile(&control_feel)?;
        tracing::info!(
            feel_profile_id = %control_feel_identity.profile_id,
            feel_schema = control_feel_identity.schema,
            feel_digest = %control_feel_identity.digest,
            "Aviate control-feel profile selected"
        );
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
            match FlightUplink::new_with_profile(control_feel) {
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
        #[cfg(feature = "sim")]
        let (frames, camera_bridge, frame_forwarder) = camera::spawn_camera_bridge().await;
        #[cfg(not(feature = "sim"))]
        let (frames, camera_bridge, frame_forwarder) = super::no_camera();
        Ok(Self {
            vehicle,
            profile,
            estimate,
            truth,
            uplink,
            control_feel_identity: Some(control_feel_identity),
            frames,
            // A flight vehicle's gimbal is a real device on its own link,
            // not a rendered view, so the pointing attachment exists only
            // in a simulation build — and only for the producer that
            // actually renders a payload view. A Gazebo session has a
            // camera but no gimbal behind it: advertising the scope
            // there is a control surface with nothing on the other end.
            #[cfg(feature = "sim")]
            pointing: (camera_bridge.is_some()
                && camera::camera_mode() == camera::CameraMode::XPlanePlugin)
                .then(super::pointing::PointingState::default),
            #[cfg(not(feature = "sim"))]
            pointing: None,
            camera_bridge,
            _frame_forwarder: frame_forwarder,
            arm: None,
            arm_incarnation,
            started_at: std::time::Instant::now(),
            last_reset: None,
            view_publish_failed: false,
            reset_latch: None,
            #[cfg(test)]
            reset_spawns: 0,
            link_loss_policy: BTreeMap::new(),
        })
    }
}

fn compatibility_control_feel() -> Result<ValidatedFlightFeelProfile, AviateAdapterError> {
    ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
        .map_err(|source| AviateAdapterError::InvalidControlFeel { source })
}

pub(super) fn validate_adapter_control_feel(
    control_feel: &ValidatedFlightFeelProfile,
) -> Result<(), AviateAdapterError> {
    if control_feel.profile().hold.require_accel {
        return Err(AviateAdapterError::UnsupportedControlFeel {
            detail: "hold.require_accel needs an acceleration source with provenance".to_owned(),
        });
    }
    Ok(())
}
