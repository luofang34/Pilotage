//! The loopback ports of one PX4 SITL instance and its host.

/// The highest PX4 instance this backend runs. PX4 shifts its MAVLink
/// ports by the instance; the host's telemetry port (14550 + instance)
/// would reach PX4's offboard ports (14580 + instance) past this.
pub(super) const MAX_INSTANCE: u16 = 29;

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
pub(super) fn system_id(instance: u16) -> u16 {
    instance + 1
}
