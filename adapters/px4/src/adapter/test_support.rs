//! Constructors that wire a PX4 adapter around test doubles.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use pilotage_mavlink::LinkState;
use pilotage_protocol::{VehicleClass, VehicleId};

use super::{EstimateSource, Px4Adapter, sampling};
use crate::uplink::Px4Uplink;

impl Px4Adapter {
    /// Wires an adapter around a caller-supplied state cache, for tests.
    pub(crate) fn from_state(vehicle: VehicleId, state: Arc<Mutex<LinkState>>) -> Self {
        Self {
            vehicle,
            vehicle_class: VehicleClass::Multicopter,
            estimate: Some(EstimateSource { state, _link: None }),
            uplink: None,
            fixed_wing: None,
            gimbal: None,
            frames: None,
            _camera_bridge: None,
            _frame_forwarder: None,
            arm: None,
            last_seen_heartbeat: None,
            arm_incarnation: pilotage_adapter_api::SourceIncarnation::new([0; 16]),
            gimbal_stamp: sampling::GimbalStamp::new(pilotage_adapter_api::SourceIncarnation::new(
                [0x60; 16],
            )),
            started_at: std::time::Instant::now(),
            last_reset: None,
            reset_latch: None,
            reset_spawns: 0,
            link_loss_policy: BTreeMap::new(),
        }
    }

    /// Installs a test uplink, for tests.
    pub(crate) fn with_uplink(mut self, uplink: Px4Uplink) -> Self {
        self.uplink = Some(uplink);
        self
    }

    /// Installs a test gimbal control path, for tests.
    pub(crate) fn with_gimbal(mut self, gimbal: crate::gimbal::Px4GimbalControl) -> Self {
        self.gimbal = Some(gimbal);
        self
    }

    /// Declares the advertised vehicle class without an airframe uplink.
    pub(crate) fn with_vehicle_class(mut self, vehicle_class: VehicleClass) -> Self {
        self.vehicle_class = vehicle_class;
        self
    }
}
