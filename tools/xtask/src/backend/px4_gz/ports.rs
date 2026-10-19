//! The loopback ports of one PX4 SITL instance and its host, the claims
//! they make, and the link environment the host reads them from. Every
//! PX4 SITL backend shares them: the simulator behind PX4 changes, its
//! MAVLink links do not.

use crate::conflicts::{Claim, Remedy};

use super::vehicle::Airframe;

/// The highest PX4 instance this backend runs. PX4 shifts its MAVLink
/// ports by the instance; the host's telemetry port (14550 + instance)
/// would reach PX4's offboard ports (14580 + instance) past this.
pub(crate) const MAX_INSTANCE: u16 = 29;

/// The loopback ports one PX4 SITL instance and its host use.
///
/// PX4 derives its own from the instance (`px4-rc.mavlink`): the GCS
/// link binds 18570 + N and the offboard link 14580 + N. The host's
/// telemetry port is this launcher's choice. PX4's GCS link sends to
/// 14550 until a peer speaks to it, then to that peer — the host's link
/// speaks first thing from its telemetry socket, so each instance's
/// stream moves to its own host. The frames that reach 14550 before that
/// carry system id N + 1, which another session's link drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Px4Ports {
    /// PX4's GCS link (stream-interval requests go here).
    pub(crate) gcs_local: u16,
    /// PX4's offboard link (setpoints and commands go here).
    pub(crate) offboard_local: u16,
    /// The host's MAVLink receive socket.
    pub(crate) host_telemetry: u16,
}

impl Px4Ports {
    /// The ports of PX4 instance `instance`.
    pub(crate) fn for_instance(instance: u16) -> Self {
        Self {
            gcs_local: 18_570 + instance,
            offboard_local: 14_580 + instance,
            host_telemetry: 14_550 + instance,
        }
    }
}

/// PX4 instance N boots as MAVLink system N + 1 (`rcS`: `MAV_SYS_ID`).
pub(crate) fn system_id(instance: u16) -> u16 {
    instance + 1
}

/// The UDP ports one PX4 instance and its host's link hold.
pub(crate) fn link_claims(instance: u16) -> Vec<Claim> {
    let ports = Px4Ports::for_instance(instance);
    vec![
        Claim::Udp {
            port: ports.gcs_local,
            holder: format!("PX4 instance {instance} GCS link"),
            remedy: Remedy::Instance,
        },
        Claim::Udp {
            port: ports.offboard_local,
            holder: format!("PX4 instance {instance} offboard link"),
            remedy: Remedy::Instance,
        },
        Claim::Udp {
            port: ports.host_telemetry,
            holder: format!("session host MAVLink link for PX4 instance {instance}"),
            remedy: Remedy::Instance,
        },
    ]
}

/// The host environment that names PX4 instance `instance`'s links, its
/// MAVLink system, and how the vehicle flies.
pub(crate) fn host_env(instance: u16, airframe: Airframe) -> Vec<(String, String)> {
    let ports = Px4Ports::for_instance(instance);
    let loopback = |port: u16| format!("127.0.0.1:{port}");
    vec![
        ("PILOTAGE_PX4_PROFILE".to_owned(), "simulation".to_owned()),
        (
            "PILOTAGE_PX4_AIRFRAME".to_owned(),
            airframe.as_env_value().to_owned(),
        ),
        (
            "PILOTAGE_PX4_ADDR".to_owned(),
            loopback(ports.host_telemetry),
        ),
        (
            "PILOTAGE_PX4_GCS_ADDR".to_owned(),
            loopback(ports.gcs_local),
        ),
        (
            "PILOTAGE_PX4_FC_ADDR".to_owned(),
            loopback(ports.offboard_local),
        ),
        (
            "PILOTAGE_PX4_SYSTEM_ID".to_owned(),
            system_id(instance).to_string(),
        ),
        // The viewer's reset runs scripts/reset-px4-sim.sh from the
        // host; this names the instance it resets.
        ("PILOTAGE_PX4_INSTANCE".to_owned(), instance.to_string()),
    ]
}
