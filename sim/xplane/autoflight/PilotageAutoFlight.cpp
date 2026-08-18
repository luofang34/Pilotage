// PilotageAutoFlight - unattended flight start for simulation sessions.
//
// The plugin does nothing unless PILOTAGE_XPLANE_ACF is set in the
// environment of the X-Plane process. A launcher that owns the X-Plane
// session sets:
//
//   PILOTAGE_XPLANE_ACF      path of the .acf file, relative to the
//                            X-Plane root (required to activate)
//   PILOTAGE_XPLANE_AIRPORT  ICAO id for the start airport (optional)
//   PILOTAGE_XPLANE_CONNECT  "1" starts the px4xplane SITL listener
//                            after the aircraft is loaded (optional)
//
// Sequence: wait for the simulator to settle, load the aircraft, wait
// for XPLM_MSG_PLANE_LOADED for the user aircraft, wait for the run
// loop to run without multi-second stalls (texture and scenery loading
// after a cold boot), then trigger px4xplane/toggleEnable so PX4 (or
// an other HIL FC) can connect without operator input. The stall gate
// matters: the px4xplane bridge disconnects on actuator-feedback
// deadline misses, so arming it during load-stall churn wedges the
// FC's lockstep clock.

#include "XPLMDefs.h"
#include "XPLMPlanes.h"
#include "XPLMPlugin.h"
#include "XPLMProcessing.h"
#include "XPLMUtilities.h"

#include <chrono>
#include <cstdlib>
#include <cstring>
#include <string>

namespace {

enum class Phase {
    Inactive,
    WaitBeforeLoad,
    WaitPlaneLoaded,
    WaitStableFrames,
    Done,
};

// Consecutive one-second callbacks that must arrive without a stall
// before the SITL listener is armed.
constexpr int kStableSecondsRequired = 8;
// A callback gap above this is a run-loop stall and restarts the count.
constexpr double kStallThresholdSeconds = 2.5;

Phase phase = Phase::Inactive;
std::string acf_path;
std::string airport_id;
bool want_connect = false;
bool loop_registered = false;
int stable_seconds = 0;
std::chrono::steady_clock::time_point last_tick;

void log_line(const std::string& text) {
    std::string line = "PilotageAutoFlight: " + text + "\n";
    XPLMDebugString(line.c_str());
}

float OnStableFramesTick() {
    auto now = std::chrono::steady_clock::now();
    double gap = std::chrono::duration<double>(now - last_tick).count();
    last_tick = now;
    if (gap > kStallThresholdSeconds) {
        if (stable_seconds > 0) {
            log_line("run-loop stall observed; restarting the stable-frame count");
        }
        stable_seconds = 0;
        return 1.0f;
    }
    stable_seconds += 1;
    if (stable_seconds < kStableSecondsRequired) {
        return 1.0f;
    }
    XPLMCommandRef cmd = XPLMFindCommand("px4xplane/toggleEnable");
    if (cmd == nullptr) {
        log_line("px4xplane/toggleEnable not found; retrying");
        return 2.0f;
    }
    log_line("run loop is stable; triggering px4xplane/toggleEnable");
    XPLMCommandOnce(cmd);
    phase = Phase::Done;
    return 0.0f;
}

float FlightLoop(float, float, int, void*) {
    switch (phase) {
        case Phase::WaitBeforeLoad: {
            log_line("loading aircraft " + acf_path);
            phase = Phase::WaitPlaneLoaded;
            // Full path is required by XPLMSetUsersAircraft.
            char system_path[1024];
            XPLMGetSystemPath(system_path);
            std::string full = std::string(system_path) + acf_path;
            XPLMSetUsersAircraft(full.c_str());
            if (!airport_id.empty()) {
                log_line("placing user at " + airport_id);
                XPLMPlaceUserAtAirport(airport_id.c_str());
            }
            return 5.0f;
        }
        case Phase::WaitPlaneLoaded:
            // XPluginReceiveMessage moves the phase forward.
            return 2.0f;
        case Phase::WaitStableFrames:
            return OnStableFramesTick();
        default:
            return 0.0f;
    }
}

}  // namespace

PLUGIN_API int XPluginStart(char* out_name, char* out_sig, char* out_desc) {
    std::strcpy(out_name, "PilotageAutoFlight");
    std::strcpy(out_sig, "systems.sokoly.pilotage.autoflight");
    std::strcpy(out_desc, "Unattended flight start for Pilotage sim sessions");

    const char* acf = std::getenv("PILOTAGE_XPLANE_ACF");
    if (acf == nullptr || acf[0] == '\0') {
        log_line("PILOTAGE_XPLANE_ACF not set; inactive");
        return 1;
    }
    acf_path = acf;
    const char* airport = std::getenv("PILOTAGE_XPLANE_AIRPORT");
    if (airport != nullptr) {
        airport_id = airport;
    }
    const char* connect = std::getenv("PILOTAGE_XPLANE_CONNECT");
    want_connect = (connect != nullptr && std::strcmp(connect, "1") == 0);
    phase = Phase::WaitBeforeLoad;
    return 1;
}

PLUGIN_API void XPluginStop(void) {}

PLUGIN_API int XPluginEnable(void) {
    if (phase != Phase::Inactive && phase != Phase::Done && !loop_registered) {
        // Let the simulator settle before the aircraft swap.
        XPLMRegisterFlightLoopCallback(FlightLoop, 10.0f, nullptr);
        loop_registered = true;
    }
    return 1;
}

PLUGIN_API void XPluginDisable(void) {
    if (loop_registered) {
        XPLMUnregisterFlightLoopCallback(FlightLoop, nullptr);
        loop_registered = false;
    }
}

PLUGIN_API void XPluginReceiveMessage(XPLMPluginID, int msg, void* param) {
    if (msg == XPLM_MSG_PLANE_LOADED && phase == Phase::WaitPlaneLoaded) {
        // param is the aircraft index; 0 is the user aircraft.
        if (reinterpret_cast<intptr_t>(param) == 0) {
            if (!want_connect) {
                log_line("user aircraft loaded");
                phase = Phase::Done;
                return;
            }
            log_line("user aircraft loaded; waiting for a stable run loop");
            stable_seconds = 0;
            last_tick = std::chrono::steady_clock::now();
            phase = Phase::WaitStableFrames;
            XPLMSetFlightLoopCallbackInterval(FlightLoop, 1.0f, 1, nullptr);
        }
    }
}
