#include "camera_state.h"

#include "XPLMUtilities.h"

#include <algorithm>
#include <cstdlib>

namespace pilotage_camera {

namespace {

float clamp_float(float value, float low, float high) {
    if (!(value == value)) {  // NaN
        return low;
    }
    return std::min(high, std::max(low, value));
}

}  // namespace

void CameraState::apply(const CameraCommand* command) {
    if (command == nullptr) {
        return;
    }
    switch (command->mode) {
        case 0:
            mode_ = CameraMode::Fpv;
            break;
        case 1:
            mode_ = CameraMode::Gimbal;
            break;
        case 2:
            mode_ = CameraMode::Free;
            break;
        default:
            // An unknown mode keeps the current one: a producer must not
            // invent a view the host did not ask for.
            break;
    }
    pan_rad_ = clamp_float(command->pan_rad, -kPanLimitRad, kPanLimitRad);
    tilt_rad_ = clamp_float(command->tilt_rad, kTiltMinRad, kTiltMaxRad);
    if (command->zoom_detent < static_cast<std::uint32_t>(kZoomDetentCount)) {
        zoom_detent_ = static_cast<int>(command->zoom_detent);
    }
}

CameraState& state() {
    static CameraState instance;
    return instance;
}

bool hud_enabled() {
    static const bool enabled = [] {
        const char* value = std::getenv("PILOTAGE_XPLANE_CAMERA_HUD");
        return value != nullptr && std::strcmp(value, "1") == 0;
    }();
    return enabled;
}

unsigned env_unsigned(const char* name, unsigned fallback) {
    const char* value = std::getenv(name);
    if (value == nullptr || value[0] == '\0') {
        return fallback;
    }
    char* end = nullptr;
    unsigned long parsed = std::strtoul(value, &end, 10);
    if (end == value || parsed == 0 || parsed > 0xFFFF'FFFFUL) {
        return fallback;
    }
    return static_cast<unsigned>(parsed);
}

void log_line(const std::string& text) {
    XPLMDebugString(("PilotageCamera: " + text + "\n").c_str());
}

}  // namespace pilotage_camera
