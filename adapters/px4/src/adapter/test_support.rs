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
            camera_control: None,
            frames: None,
            _camera_bridge: None,
            _frame_forwarder: None,
            arm: pilotage_mavlink::sampling::ArmTracker::default(),
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
    pub(crate) fn with_gimbal(mut self, gimbal: pilotage_mavlink::ManagerControl) -> Self {
        let state = self.estimate.as_ref().expect("test state").state.clone();
        super::gimbal_tests::discover_gimbal(&state);
        let report = state
            .lock()
            .expect("state")
            .gimbals
            .managers
            .get(&(1, 2))
            .and_then(|report| report.status)
            .map(|(status, _)| status)
            .expect("manager");
        pilotage_mavlink::link::apply_messages_at(
            &state,
            &[(
                super::tests::SOURCE,
                pilotage_mavlink::codec::FcMessage::GimbalManagerStatus(
                    pilotage_mavlink::codec::GimbalManagerStatus {
                        time_boot_ms: report.time_boot_ms.wrapping_add(1),
                        ..report
                    },
                ),
            )],
            0,
            0,
            std::time::Instant::now(),
        );
        let mut gimbal = gimbal.with_manager_state(state);
        gimbal.refresh_discovery();
        self.gimbal = Some(gimbal);
        self
    }

    /// Declares the advertised vehicle class without an airframe uplink.
    pub(crate) fn with_vehicle_class(mut self, vehicle_class: VehicleClass) -> Self {
        self.vehicle_class = vehicle_class;
        self
    }
}
