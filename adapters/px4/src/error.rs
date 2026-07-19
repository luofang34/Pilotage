//! Error type for the PX4 adapter.

/// Why the PX4 adapter could not start.
#[derive(Debug, thiserror::Error)]
pub enum Px4AdapterError {
    /// The MAVLink receive link could not start.
    #[error(transparent)]
    Link(#[from] pilotage_mavlink::LinkError),
    /// The command uplink socket could not be bound.
    #[error("binding the PX4 command uplink socket failed: {source}")]
    UplinkBind {
        /// The underlying socket error.
        #[source]
        source: std::io::Error,
    },
}
