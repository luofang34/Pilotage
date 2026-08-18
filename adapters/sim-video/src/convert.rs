//! Boundary conversion from the sidecar wire frame to the engine-neutral
//! unstamped frame the `pilotage-adapter-api` stamper consumes.

use pilotage_adapter_api::UnstampedFrame;

use crate::wire::BridgeFrame;

impl From<BridgeFrame> for UnstampedFrame {
    fn from(frame: BridgeFrame) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
            pixel_format: frame.pixel_format,
            time_ns: frame.sim_time_ns,
            rgb: frame.rgb,
            camera_id: frame.camera_id,
        }
    }
}
