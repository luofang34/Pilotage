// PilotageCamera - X-Plane vehicle camera export for Pilotage.
//
// Owns the simulator's single rendered view as a VEHICLE camera (FPV or
// gimbal payload), captures each rendered frame from the plugin bridge
// framebuffer, and streams it to the Pilotage host as length-delimited
// `pilotage.bridge.v1` envelopes over a localhost TCP connection it
// dials, exactly like every other Pilotage video producer. The host
// steers mode, gimbal angles, and zoom over the same socket.
//
// Capture mechanism: `glReadPixels` from a 2-D drawing phase reads the
// rendered scene. Laminar documents this for the 11.50+ plugin bridge
// ("plugins can read back the frame buffer from any 2-d drawing
// callback"); no API renders the world offscreen for a plugin. The path
// is therefore VALIDATED, NOT GUARANTEED: it is verified against a
// recorded X-Plane build, and the producer fails loud (stops streaming,
// logs once) when readback stops yielding a live scene, rather than
// streaming a frozen or blank picture.
//
// HARD CONSTRAINT: never touch `GL_FRAMEBUFFER_SRGB`. Toggling it under
// the bridge driver makes all later rendering disappear. The sRGB state
// belongs to X-Plane.
//
// Environment:
//   PILOTAGE_XPLANE_CAMERA_PORT  host listener port (default 45990)
//   PILOTAGE_XPLANE_CAMERA_FPS   capture rate cap (default 24)
//   PILOTAGE_XPLANE_CAMERA_HUD   "1" draws the producer HUD overlay

#include "XPLMCamera.h"
#include "XPLMDataAccess.h"
#include "XPLMDefs.h"
#include "XPLMDisplay.h"
#include "XPLMGraphics.h"
#include "XPLMPlugin.h"
#include "XPLMProcessing.h"
#include "XPLMUtilities.h"

#include <OpenGL/gl.h>
#include <OpenGL/glext.h>

#include <cmath>
#include <cstdio>
#include <cstring>
#include <string>

#include "camera_state.h"
#include "capture.h"
#include "hud.h"
#include "link.h"
#include "view.h"

namespace pilotage_camera {
namespace {

XPLMFlightLoopID pump_loop = nullptr;
bool draw_registered = false;

float PumpLoop(float, float, int, void*) {
    link().pump();
    if (link().connected()) {
        state().apply(link().take_command());
    }
    view().reassert_if_lost();
    return -1.0F;
}

int OnWindowPhase(XPLMDrawingPhase, int, void*) {
    if (hud_enabled() && state().mode() != CameraMode::Free) {
        draw_hud(state());
    }
    capture().on_frame(link());
    return 1;
}

}  // namespace
}  // namespace pilotage_camera

PLUGIN_API int XPluginStart(char* out_name, char* out_sig, char* out_desc) {
    std::strcpy(out_name, "PilotageCamera");
    std::strcpy(out_sig, "systems.sokoly.pilotage.camera");
    std::strcpy(out_desc, "Vehicle camera export for Pilotage sessions");
    return 1;
}

PLUGIN_API void XPluginStop(void) {}

PLUGIN_API int XPluginEnable(void) {
    using namespace pilotage_camera;
    view().start();
    XPLMCreateFlightLoop_t loop = {
        sizeof(XPLMCreateFlightLoop_t),
        xplm_FlightLoop_Phase_AfterFlightModel,
        PumpLoop,
        nullptr,
    };
    pump_loop = XPLMCreateFlightLoop(&loop);
    XPLMScheduleFlightLoop(pump_loop, -1.0F, 1);
    XPLMRegisterDrawCallback(OnWindowPhase, xplm_Phase_Window, 0, nullptr);
    draw_registered = true;
    log_line("enabled");
    return 1;
}

PLUGIN_API void XPluginDisable(void) {
    using namespace pilotage_camera;
    if (draw_registered) {
        XPLMUnregisterDrawCallback(OnWindowPhase, xplm_Phase_Window, 0,
                                   nullptr);
        draw_registered = false;
    }
    if (pump_loop != nullptr) {
        XPLMDestroyFlightLoop(pump_loop);
        pump_loop = nullptr;
    }
    capture().shutdown();
    link().close();
    // Releasing the view restores the operator's own camera and the
    // field of view this plugin borrowed.
    view().stop();
    log_line("disabled");
}

PLUGIN_API void XPluginReceiveMessage(XPLMPluginID, int msg, void* param) {
    using namespace pilotage_camera;
    if (msg == XPLM_MSG_PLANE_LOADED &&
        reinterpret_cast<intptr_t>(param) == 0) {
        // A flight reload drops the camera control; take it back so a
        // simulation reset does not silently end the vehicle view.
        view().start();
        capture().reset_epoch();
    }
}
