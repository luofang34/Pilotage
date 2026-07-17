//! Role-bound source construction (LINK-04): which links a profile
//! binds, and the provenance stamping for FC-owned state reports.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use pilotage_adapter_api::{
    FcStateSample, MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity,
    SourceRole,
};

use super::AviateProfile;
use super::shm_sampling::ShmSource;
use crate::error::AviateAdapterError;
use crate::incarnation::IncarnationProvider;
use crate::link::{AviateLink, LatestAviate, LinkConfig};

/// The MAVLink estimate link: the latest-value cache plus the receive
/// task that feeds it. This link only ever produces the FC operational
/// estimate role.
#[derive(Debug)]
pub(super) struct EstimateSource {
    pub(super) state: Arc<Mutex<LatestAviate>>,
    // Kept alive for its receive task; dropped with the adapter.
    pub(super) _link: Option<AviateLink>,
}

/// One FC arm report received on the uplink socket, with receive-side
/// acquisition metadata for its provenance stamp (the heartbeat wire
/// carries no source timestamp).
#[derive(Debug, Clone, Copy)]
pub(super) struct ArmReport {
    pub(super) armed: bool,
    /// MAVLink system id of the FC that reported, as configured on the
    /// accepting uplink.
    pub(super) system_id: u8,
    /// MAVLink component id of the reporting FC.
    pub(super) component_id: u8,
    pub(super) sequence: u32,
    pub(super) acquired_at: Instant,
}

/// Binds the links a profile's roles require. Every role gets its own
/// attachment identity: truth, estimate, and FC state reports are
/// independent sources, not one link.
pub(super) async fn bind_sources<P: IncarnationProvider>(
    profile: AviateProfile,
    config: LinkConfig,
    provider: &mut P,
) -> Result<(Option<EstimateSource>, Option<Box<ShmSource>>), AviateAdapterError> {
    match profile {
        AviateProfile::Physical => {
            let incarnation = provider.next_incarnation_blocking()?;
            Ok((Some(estimate_source(config, incarnation).await?), None))
        }
        AviateProfile::Simulation => {
            let truth = match provider
                .next_incarnation_blocking()
                .and_then(|incarnation| ShmSource::open(0, incarnation))
            {
                Ok(source) => {
                    tracing::info!("Aviate simulation-truth oracle: shared-memory block");
                    Some(Box::new(source))
                }
                Err(error) => {
                    tracing::info!(%error, "truth oracle not attachable; estimate-only simulation");
                    None
                }
            };
            let incarnation = provider.next_incarnation_blocking()?;
            Ok((Some(estimate_source(config, incarnation).await?), truth))
        }
        AviateProfile::OracleOnly => {
            let incarnation = provider.next_incarnation_blocking()?;
            Ok((None, Some(Box::new(ShmSource::open(0, incarnation)?))))
        }
    }
}

async fn estimate_source(
    config: LinkConfig,
    incarnation: SourceIncarnation,
) -> Result<EstimateSource, AviateAdapterError> {
    let link = AviateLink::start(config, incarnation).await?;
    Ok(EstimateSource {
        state: link.state(),
        _link: Some(link),
    })
}

/// The latest FC arm report as a stamped sample, or `None` before the FC
/// has reported. Unknown arm state is expressed by absence, never by a
/// fabricated stamp.
pub(super) fn fc_state_sample(
    report: Option<ArmReport>,
    incarnation: SourceIncarnation,
    started_at: Instant,
) -> Option<FcStateSample> {
    let report = report?;
    let acquired = report
        .acquired_at
        .checked_duration_since(started_at)
        .unwrap_or_default();
    Some(FcStateSample {
        arm_state: if report.armed { 2 } else { 1 },
        stamp: MeasurementStamp {
            // Role is the discriminator; the id carries the configured
            // FC identity: (system id << 8) | component id.
            role: SourceRole::FcState,
            // MAVLink frames are CRC-checked but unsigned: checksummed,
            // never authenticated.
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: (u64::from(report.system_id) << 8) | u64::from(report.component_id),
            source_incarnation: incarnation,
            // A gateway-generated attachment identity cannot observe an
            // FC restart; a source-issued boot/session identity replaces
            // this constant once the FC publishes one.
            source_epoch: 1,
            sequence: report.sequence,
            acquired_at_ns: u64::try_from(acquired.as_nanos()).unwrap_or(u64::MAX),
            clock: MeasurementClock::HostMonotonic,
        },
    })
}
