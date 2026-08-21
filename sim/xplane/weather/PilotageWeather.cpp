// Deterministic weather control for Pilotage simulation trials.

#include "XPLMDataAccess.h"
#include "XPLMPlugin.h"
#include "XPLMProcessing.h"

#include "weather_state.h"

#include <algorithm>
#include <array>
#include <cstring>
#include <string>

namespace {

using pilotage::weather::IsValidRequest;
using pilotage::weather::RequestStatus;
using pilotage::weather::WeatherRequest;

constexpr int kMaximumWindLayers = 13;

WeatherRequest requested;
float applied_generation = -1.0F;
float status = static_cast<float>(RequestStatus::Idle);
float actual_speed_mps = 0.0F;
float actual_direction_deg = 0.0F;
bool loop_registered = false;

std::array<XPLMDataRef, 8> custom_refs{};
XPLMDataRef update_immediately_ref = nullptr;
XPLMDataRef change_mode_ref = nullptr;
XPLMDataRef variability_ref = nullptr;
XPLMDataRef wind_speed_ref = nullptr;
XPLMDataRef wind_direction_ref = nullptr;
XPLMDataRef turbulence_ref = nullptr;
XPLMDataRef actual_speed_ref = nullptr;
XPLMDataRef actual_direction_ref = nullptr;

float ReadFloat(void* refcon) {
    return *static_cast<float*>(refcon);
}

void WriteFloat(void* refcon, float value) {
    *static_cast<float*>(refcon) = value;
}

XPLMDataRef RegisterFloat(const char* name, bool writable, float* value) {
    return XPLMRegisterDataAccessor(
        name, xplmType_Float, writable ? 1 : 0, nullptr, nullptr, ReadFloat,
        writable ? WriteFloat : nullptr, nullptr, nullptr, nullptr, nullptr,
        nullptr, nullptr, nullptr, nullptr, value, value);
}

void RegisterCustomDatarefs() {
    custom_refs[0] = RegisterFloat("pilotage/weather/wind_speed_mps", true,
                                   &requested.wind_speed_mps);
    custom_refs[1] = RegisterFloat("pilotage/weather/wind_direction_deg", true,
                                   &requested.wind_direction_deg);
    custom_refs[2] = RegisterFloat("pilotage/weather/turbulence", true,
                                   &requested.turbulence);
    custom_refs[3] = RegisterFloat("pilotage/weather/apply_generation", true,
                                   &requested.generation);
    custom_refs[4] = RegisterFloat("pilotage/weather/applied_generation", false,
                                   &applied_generation);
    custom_refs[5] =
        RegisterFloat("pilotage/weather/status", false, &status);
    custom_refs[6] = RegisterFloat("pilotage/weather/actual_speed_mps", false,
                                   &actual_speed_mps);
    custom_refs[7] = RegisterFloat("pilotage/weather/actual_direction_deg", false,
                                   &actual_direction_deg);
}

void BindSimulatorDatarefs() {
    update_immediately_ref =
        XPLMFindDataRef("sim/weather/region/update_immediately");
    change_mode_ref = XPLMFindDataRef("sim/weather/region/change_mode");
    variability_ref = XPLMFindDataRef("sim/weather/region/variability_pct");
    wind_speed_ref = XPLMFindDataRef("sim/weather/region/wind_speed_msc");
    wind_direction_ref =
        XPLMFindDataRef("sim/weather/region/wind_direction_degt");
    turbulence_ref = XPLMFindDataRef("sim/weather/region/turbulence");
    actual_speed_ref =
        XPLMFindDataRef("sim/weather/aircraft/wind_now_speed_msc");
    actual_direction_ref =
        XPLMFindDataRef("sim/weather/aircraft/wind_now_direction_degt");
}

bool RequiredDatarefsExist() {
    return update_immediately_ref != nullptr && change_mode_ref != nullptr &&
           variability_ref != nullptr && wind_speed_ref != nullptr &&
           wind_direction_ref != nullptr && turbulence_ref != nullptr;
}

void ApplyRequest() {
    if (!IsValidRequest(requested)) {
        status = static_cast<float>(RequestStatus::Invalid);
        return;
    }
    if (!RequiredDatarefsExist()) {
        status = static_cast<float>(RequestStatus::DatarefMissing);
        return;
    }
    const int layer_count = std::min(
        {kMaximumWindLayers, XPLMGetDatavf(wind_speed_ref, nullptr, 0, 0),
         XPLMGetDatavf(wind_direction_ref, nullptr, 0, 0),
         XPLMGetDatavf(turbulence_ref, nullptr, 0, 0)});
    if (layer_count <= 0) {
        status = static_cast<float>(RequestStatus::DatarefMissing);
        return;
    }
    std::array<float, kMaximumWindLayers> speed{};
    std::array<float, kMaximumWindLayers> direction{};
    std::array<float, kMaximumWindLayers> turbulence{};
    speed.fill(requested.wind_speed_mps);
    direction.fill(requested.wind_direction_deg);
    turbulence.fill(requested.turbulence);

    XPLMSetDatai(change_mode_ref, 3);
    XPLMSetDataf(variability_ref, 0.0F);
    XPLMSetDatai(update_immediately_ref, 1);
    XPLMSetDatavf(wind_speed_ref, speed.data(), 0, layer_count);
    XPLMSetDatavf(wind_direction_ref, direction.data(), 0, layer_count);
    XPLMSetDatavf(turbulence_ref, turbulence.data(), 0, layer_count);
    applied_generation = requested.generation;
    status = static_cast<float>(RequestStatus::Applied);
}

float FlightLoop(float, float, int, void*) {
    if (requested.generation != applied_generation) {
        ApplyRequest();
    }
    if (actual_speed_ref != nullptr) {
        actual_speed_mps = XPLMGetDataf(actual_speed_ref);
    }
    if (actual_direction_ref != nullptr) {
        actual_direction_deg = XPLMGetDataf(actual_direction_ref);
    }
    return -1.0F;
}

}  // namespace

PLUGIN_API int XPluginStart(char* out_name, char* out_sig, char* out_desc) {
    std::strcpy(out_name, "PilotageWeather");
    std::strcpy(out_sig, "systems.sokoly.pilotage.weather");
    std::strcpy(out_desc, "Deterministic weather control for Pilotage trials");
    RegisterCustomDatarefs();
    return 1;
}

PLUGIN_API void XPluginStop(void) {
    if (loop_registered) {
        XPLMUnregisterFlightLoopCallback(FlightLoop, nullptr);
        loop_registered = false;
    }
    for (XPLMDataRef ref : custom_refs) {
        if (ref != nullptr) {
            XPLMUnregisterDataAccessor(ref);
        }
    }
}

PLUGIN_API int XPluginEnable(void) {
    BindSimulatorDatarefs();
    if (!loop_registered) {
        XPLMRegisterFlightLoopCallback(FlightLoop, -1.0F, nullptr);
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

PLUGIN_API void XPluginReceiveMessage(XPLMPluginID, int, void*) {}
