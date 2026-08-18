// The optional producer HUD: a minimal payload-camera reticle and mode
// legend, drawn into the picture BEFORE capture, so it travels with the
// frame the way a camera's own overlay does.
//
// This is the camera's own overlay, not a flight display: the operator
// client draws its own conformal symbology from telemetry, gated on a
// recognized calibration. Enabled by `PILOTAGE_XPLANE_CAMERA_HUD=1`.

#ifndef PILOTAGE_CAMERA_HUD_H
#define PILOTAGE_CAMERA_HUD_H

#include "camera_state.h"

namespace pilotage_camera {

void draw_hud(const CameraState& state);

}  // namespace pilotage_camera

#endif  // PILOTAGE_CAMERA_HUD_H
