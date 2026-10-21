//! Camera routing changes use the profile's neutral handover.

use crate::{CameraPanRouting, ControlRuntime};

impl ControlRuntime {
    /// Change the camera pan destination through a neutral mapping handover.
    /// Returns false when the declaration is unchanged.
    pub fn set_camera_pan_routing(&mut self, routing: CameraPanRouting) -> bool {
        if self.camera_pan == routing {
            return false;
        }
        self.camera_pan_handover();
        self.camera_pan = routing;
        self.reseed_edge_baselines();
        true
    }

    pub(super) fn camera_pan_handover(&mut self) {
        // A route update must not replace a scheme already waiting for neutral.
        if self.pending.is_none() {
            self.reactivate_mapping();
        }
    }

    /// The configured pan destination. Authority still gates the actual output.
    #[must_use]
    pub const fn camera_pan_routing(&self) -> CameraPanRouting {
        self.camera_pan
    }
}
