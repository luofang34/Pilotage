//! Where a simulation session's truth comes from: the shm oracle, or the
//! host's own lane with no oracle at all.

use crate::adapter::sources::bind_sources;
use crate::adapter::{AviateProfile, AviateTruth};
use crate::incarnation::OsIncarnationProvider;

/// A session whose host carries truth on its own lane attaches no shm
/// oracle, so another simulator's block on the machine (Gazebo writes
/// instance 0's) never stands in for this vehicle; an oracle-only session
/// told the same has nothing to carry and refuses to start.
#[tokio::test]
async fn a_host_lane_session_attaches_no_shm_oracle() {
    let config = pilotage_mavlink::link::LinkConfig {
        endpoint: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        ..pilotage_mavlink::link::LinkConfig::simulator()
    };
    let mut provider = OsIncarnationProvider;
    let (estimate, truth) = bind_sources(
        AviateProfile::Simulation,
        AviateTruth::HostLane,
        config,
        &mut provider,
    )
    .await
    .expect("simulation binds its estimate");
    assert!(estimate.is_some());
    assert!(truth.is_none(), "no shm oracle on a host-lane session");
    assert!(matches!(
        bind_sources(
            AviateProfile::OracleOnly,
            AviateTruth::HostLane,
            config,
            &mut provider,
        )
        .await,
        Err(crate::AviateAdapterError::OracleOnlyWithoutOracle)
    ));
}
