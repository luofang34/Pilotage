// Transactional weather control for Pilotage simulation trials.

#include "XPLMDataAccess.h"
#include "XPLMPlugin.h"
#include "XPLMProcessing.h"
#include "XPLMUtilities.h"

#include "weather_state.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstring>
#include <limits>

namespace {

using pilotage::weather::CalmPayload;
using pilotage::weather::PendingWeatherRequest;
using pilotage::weather::RequestStatus;
using pilotage::weather::WeatherOperation;
using pilotage::weather::WeatherPayload;
using pilotage::weather::WeatherTransactionState;

constexpr int kMaximumWindLayers = 13;
constexpr float kReadbackTolerance = 0.0001F;
constexpr float kProtocolVersion = 1.0F;

WeatherPayload staged;
WeatherTransactionState transaction;
float apply_generation = 0.0F;
float clear_generation = 0.0F;
float actual_speed_mps = 0.0F;
float actual_direction_deg = 0.0F;
float actual_vertical_mps = 0.0F;
float actual_turbulence_profile_max = 0.0F;
bool loop_registered = false;
bool simulator_refs_bound = false;

std::array<XPLMDataRef, 18> custom_refs{};
XPLMDataRef update_immediately_ref = nullptr;
XPLMDataRef change_mode_ref = nullptr;
XPLMDataRef variability_ref = nullptr;
XPLMDataRef wind_speed_ref = nullptr;
XPLMDataRef wind_direction_ref = nullptr;
XPLMDataRef shear_speed_ref = nullptr;
XPLMDataRef shear_direction_ref = nullptr;
XPLMDataRef turbulence_ref = nullptr;
XPLMDataRef thermal_rate_ref = nullptr;
XPLMDataRef actual_speed_ref = nullptr;
XPLMDataRef actual_direction_ref = nullptr;
XPLMDataRef actual_vertical_ref = nullptr;
XPLMDataRef actual_turbulence_ref = nullptr;

float ReadFloat(void* refcon) {
    return *static_cast<float*>(refcon);
}

void WriteFloat(void* refcon, float value) {
    *static_cast<float*>(refcon) = value;
}

void WriteApplyGeneration(void*, float value) {
    apply_generation = value;
    transaction.TriggerApply(value, staged);
}

void WriteClearGeneration(void*, float value) {
    clear_generation = value;
    transaction.TriggerClear(value);
}

float ReadExpectedGeneration(void*) {
    return static_cast<float>(transaction.expected_generation());
}

float ReadResponseGeneration(void*) {
    return static_cast<float>(transaction.response_generation());
}

float ReadAppliedGeneration(void*) {
    return static_cast<float>(transaction.applied_generation());
}

float ReadResponseOperation(void*) {
    return static_cast<float>(transaction.response_operation());
}

float ReadResponseWindSpeed(void*) {
    return transaction.response_payload().wind_speed_mps;
}

float ReadResponseWindDirection(void*) {
    return transaction.response_payload().wind_direction_deg;
}

float ReadResponseTurbulence(void*) {
    return transaction.response_payload().turbulence_scale;
}

float ReadStatus(void*) {
    return static_cast<float>(transaction.status());
}

float ReadProtocolVersion(void*) { return kProtocolVersion; }

XPLMDataRef RegisterFloatValue(const char* name, bool writable, float* value,
                               XPLMSetDataf_f writer = WriteFloat) {
    return XPLMRegisterDataAccessor(
        name, xplmType_Float, writable ? 1 : 0, nullptr, nullptr, ReadFloat,
        writable ? writer : nullptr, nullptr, nullptr, nullptr, nullptr,
        nullptr, nullptr, nullptr, nullptr, value, value);
}

XPLMDataRef RegisterFloatReader(const char* name, XPLMGetDataf_f reader) {
    return XPLMRegisterDataAccessor(
        name, xplmType_Float, 0, nullptr, nullptr, reader, nullptr, nullptr,
        nullptr, nullptr, nullptr, nullptr, nullptr, nullptr, nullptr, nullptr,
        nullptr);
}

void RegisterCustomDatarefs() {
    custom_refs[0] = RegisterFloatValue(
        "pilotage/weather/wind_speed_mps", true, &staged.wind_speed_mps);
    custom_refs[1] = RegisterFloatValue(
        "pilotage/weather/wind_direction_deg", true,
        &staged.wind_direction_deg);
    custom_refs[2] = RegisterFloatValue(
        "pilotage/weather/turbulence_scale", true,
        &staged.turbulence_scale);
    custom_refs[3] = RegisterFloatValue(
        "pilotage/weather/apply_generation", true, &apply_generation,
        WriteApplyGeneration);
    custom_refs[4] = RegisterFloatValue(
        "pilotage/weather/clear_generation", true, &clear_generation,
        WriteClearGeneration);
    custom_refs[5] = RegisterFloatReader(
        "pilotage/weather/expected_generation", ReadExpectedGeneration);
    custom_refs[6] = RegisterFloatReader(
        "pilotage/weather/response_generation", ReadResponseGeneration);
    custom_refs[7] = RegisterFloatReader(
        "pilotage/weather/applied_generation", ReadAppliedGeneration);
    custom_refs[8] = RegisterFloatReader(
        "pilotage/weather/response_operation", ReadResponseOperation);
    custom_refs[9] = RegisterFloatReader(
        "pilotage/weather/response_wind_speed_mps", ReadResponseWindSpeed);
    custom_refs[10] = RegisterFloatReader(
        "pilotage/weather/response_wind_direction_deg",
        ReadResponseWindDirection);
    custom_refs[11] = RegisterFloatReader(
        "pilotage/weather/response_turbulence_scale",
        ReadResponseTurbulence);
    custom_refs[12] =
        RegisterFloatReader("pilotage/weather/status", ReadStatus);
    custom_refs[13] = RegisterFloatValue(
        "pilotage/weather/actual_speed_mps", false, &actual_speed_mps);
    custom_refs[14] = RegisterFloatValue(
        "pilotage/weather/actual_direction_deg", false,
        &actual_direction_deg);
    custom_refs[15] = RegisterFloatValue(
        "pilotage/weather/actual_vertical_mps", false, &actual_vertical_mps);
    custom_refs[16] = RegisterFloatValue(
        "pilotage/weather/actual_turbulence_profile_max", false,
        &actual_turbulence_profile_max);
    custom_refs[17] = RegisterFloatReader(
        "pilotage/weather/protocol_version", ReadProtocolVersion);
}

void BindSimulatorDatarefs() {
    update_immediately_ref =
        XPLMFindDataRef("sim/weather/region/update_immediately");
    change_mode_ref = XPLMFindDataRef("sim/weather/region/change_mode");
    variability_ref = XPLMFindDataRef("sim/weather/region/variability_pct");
    wind_speed_ref = XPLMFindDataRef("sim/weather/region/wind_speed_msc");
    wind_direction_ref =
        XPLMFindDataRef("sim/weather/region/wind_direction_degt");
    shear_speed_ref = XPLMFindDataRef("sim/weather/region/shear_speed_msc");
    shear_direction_ref =
        XPLMFindDataRef("sim/weather/region/shear_direction_degt");
    turbulence_ref = XPLMFindDataRef("sim/weather/region/turbulence");
    thermal_rate_ref = XPLMFindDataRef("sim/weather/region/thermal_rate_ms");
    actual_speed_ref =
        XPLMFindDataRef("sim/weather/aircraft/wind_now_speed_msc");
    actual_direction_ref =
        XPLMFindDataRef("sim/weather/aircraft/wind_now_direction_degt");
    actual_vertical_ref =
        XPLMFindDataRef("sim/weather/aircraft/wind_now_y_msc");
    actual_turbulence_ref =
        XPLMFindDataRef("sim/weather/aircraft/turbulence");
    simulator_refs_bound = true;
}

bool HasType(XPLMDataRef dataref, XPLMDataTypeID type) {
    return dataref != nullptr && (XPLMGetDataRefTypes(dataref) & type) != 0;
}

bool IsWritableScalar(XPLMDataRef dataref, XPLMDataTypeID type) {
    return HasType(dataref, type) && XPLMCanWriteDataRef(dataref) != 0;
}

bool IsWritableWindArray(XPLMDataRef dataref) {
    return IsWritableScalar(dataref, xplmType_FloatArray) &&
           XPLMGetDatavf(dataref, nullptr, 0, 0) == kMaximumWindLayers;
}

bool RegionDatarefsAreUsable() {
    return IsWritableScalar(update_immediately_ref, xplmType_Int) &&
           IsWritableScalar(change_mode_ref, xplmType_Int) &&
           IsWritableScalar(variability_ref, xplmType_Float) &&
           IsWritableWindArray(wind_speed_ref) &&
           IsWritableWindArray(wind_direction_ref) &&
           IsWritableWindArray(shear_speed_ref) &&
           IsWritableWindArray(shear_direction_ref) &&
           IsWritableWindArray(turbulence_ref) &&
           IsWritableScalar(thermal_rate_ref, xplmType_Float);
}

bool ActualDatarefsAreUsable() {
    return HasType(actual_speed_ref, xplmType_Float) &&
           HasType(actual_direction_ref, xplmType_Float) &&
           HasType(actual_vertical_ref, xplmType_Float) &&
           HasType(actual_turbulence_ref, xplmType_FloatArray) &&
           XPLMGetDatavf(actual_turbulence_ref, nullptr, 0, 0) ==
               kMaximumWindLayers;
}

bool NearlyEqual(float first, float second) {
    return std::fabs(first - second) <= kReadbackTolerance;
}

bool DirectionMatches(float actual, float requested) {
    if (NearlyEqual(actual, requested)) {
        return true;
    }
    return (NearlyEqual(actual, 0.0F) && NearlyEqual(requested, 360.0F)) ||
           (NearlyEqual(actual, 360.0F) && NearlyEqual(requested, 0.0F));
}

RequestStatus WriteRegionWeather(const WeatherPayload& payload) {
    if (!RegionDatarefsAreUsable()) {
        return RequestStatus::DatarefMissing;
    }

    std::array<float, kMaximumWindLayers> speed{};
    std::array<float, kMaximumWindLayers> direction{};
    std::array<float, kMaximumWindLayers> shear{};
    std::array<float, kMaximumWindLayers> turbulence{};
    speed.fill(payload.wind_speed_mps);
    direction.fill(payload.wind_direction_deg);
    turbulence.fill(payload.turbulence_scale);

    XPLMSetDatai(change_mode_ref, 3);
    XPLMSetDataf(variability_ref, 0.0F);
    XPLMSetDatai(update_immediately_ref, 1);
    XPLMSetDatavf(wind_speed_ref, speed.data(), 0, kMaximumWindLayers);
    XPLMSetDatavf(wind_direction_ref, direction.data(), 0,
                  kMaximumWindLayers);
    XPLMSetDatavf(shear_speed_ref, shear.data(), 0, kMaximumWindLayers);
    XPLMSetDatavf(shear_direction_ref, shear.data(), 0,
                  kMaximumWindLayers);
    XPLMSetDatavf(turbulence_ref, turbulence.data(), 0,
                  kMaximumWindLayers);
    XPLMSetDataf(thermal_rate_ref, 0.0F);

    std::array<float, kMaximumWindLayers> read_speed{};
    std::array<float, kMaximumWindLayers> read_direction{};
    std::array<float, kMaximumWindLayers> read_shear_speed{};
    std::array<float, kMaximumWindLayers> read_shear_direction{};
    std::array<float, kMaximumWindLayers> read_turbulence{};
    if (XPLMGetDatavf(wind_speed_ref, read_speed.data(), 0,
                      kMaximumWindLayers) != kMaximumWindLayers ||
        XPLMGetDatavf(wind_direction_ref, read_direction.data(), 0,
                      kMaximumWindLayers) != kMaximumWindLayers ||
        XPLMGetDatavf(shear_speed_ref, read_shear_speed.data(), 0,
                      kMaximumWindLayers) != kMaximumWindLayers ||
        XPLMGetDatavf(shear_direction_ref, read_shear_direction.data(), 0,
                      kMaximumWindLayers) != kMaximumWindLayers ||
        XPLMGetDatavf(turbulence_ref, read_turbulence.data(), 0,
                      kMaximumWindLayers) != kMaximumWindLayers) {
        return RequestStatus::ReadbackMismatch;
    }

    for (int index = 0; index < kMaximumWindLayers; ++index) {
        if (!NearlyEqual(read_speed[index], payload.wind_speed_mps) ||
            !DirectionMatches(read_direction[index],
                              payload.wind_direction_deg) ||
            !NearlyEqual(read_shear_speed[index], 0.0F) ||
            !NearlyEqual(read_shear_direction[index], 0.0F) ||
            !NearlyEqual(read_turbulence[index],
                         payload.turbulence_scale)) {
            return RequestStatus::ReadbackMismatch;
        }
    }
    if (XPLMGetDatai(update_immediately_ref) != 1 ||
        XPLMGetDatai(change_mode_ref) != 3 ||
        !NearlyEqual(XPLMGetDataf(variability_ref), 0.0F) ||
        !NearlyEqual(XPLMGetDataf(thermal_rate_ref), 0.0F)) {
        return RequestStatus::ReadbackMismatch;
    }
    return RequestStatus::Applied;
}

void ApplyPendingRequest() {
    PendingWeatherRequest request;
    if (!transaction.TakePending(&request)) {
        return;
    }
    const bool actual_datarefs_usable = ActualDatarefsAreUsable();
    if (request.operation == WeatherOperation::Apply &&
        !actual_datarefs_usable) {
        transaction.Fail(request, RequestStatus::DatarefMissing);
        return;
    }
    const RequestStatus result = WriteRegionWeather(request.payload);
    if (result == RequestStatus::Applied) {
        if (request.operation == WeatherOperation::Clear) {
            staged = CalmPayload();
            if (!actual_datarefs_usable) {
                transaction.Fail(request, RequestStatus::DatarefMissing);
                return;
            }
        }
        transaction.Complete(request);
    } else {
        transaction.Fail(request, result);
    }
}

void ReadActualWeather() {
    if (actual_speed_ref != nullptr) {
        actual_speed_mps = XPLMGetDataf(actual_speed_ref);
    }
    if (actual_direction_ref != nullptr) {
        actual_direction_deg = XPLMGetDataf(actual_direction_ref);
    }
    if (actual_vertical_ref != nullptr) {
        actual_vertical_mps = XPLMGetDataf(actual_vertical_ref);
    }
    std::array<float, kMaximumWindLayers> turbulence{};
    actual_turbulence_profile_max =
        std::numeric_limits<float>::quiet_NaN();
    if (actual_turbulence_ref != nullptr &&
        XPLMGetDatavf(actual_turbulence_ref, turbulence.data(), 0,
                      kMaximumWindLayers) == kMaximumWindLayers) {
        actual_turbulence_profile_max = 0.0F;
        for (float value : turbulence) {
            if (!std::isfinite(value) || value < 0.0F ||
                value > pilotage::weather::kMaximumTurbulenceScale) {
                actual_turbulence_profile_max =
                    std::numeric_limits<float>::quiet_NaN();
                break;
            }
            actual_turbulence_profile_max =
                std::max(actual_turbulence_profile_max, value);
        }
    }
}

void ClearForUnload() {
    if (simulator_refs_bound &&
        WriteRegionWeather(CalmPayload()) != RequestStatus::Applied) {
        XPLMDebugString(
            "PilotageWeather: weather clear readback failed during unload\n");
    }
    staged = CalmPayload();
    apply_generation = 0.0F;
    clear_generation = 0.0F;
    transaction.Disable();
}

float FlightLoop(float, float, int, void*) {
    ApplyPendingRequest();
    ReadActualWeather();
    return -1.0F;
}

}  // namespace

PLUGIN_API int XPluginStart(char* out_name, char* out_sig, char* out_desc) {
    std::strcpy(out_name, "PilotageWeather");
    std::strcpy(out_sig, "systems.sokoly.pilotage.weather");
    std::strcpy(out_desc, "Transactional weather control for Pilotage trials");
    transaction.Disable();
    RegisterCustomDatarefs();
    return 1;
}

PLUGIN_API void XPluginStop(void) {
    if (loop_registered) {
        XPLMUnregisterFlightLoopCallback(FlightLoop, nullptr);
        loop_registered = false;
    }
    ClearForUnload();
    for (XPLMDataRef ref : custom_refs) {
        if (ref != nullptr) {
            XPLMUnregisterDataAccessor(ref);
        }
    }
}

PLUGIN_API int XPluginEnable(void) {
    BindSimulatorDatarefs();
    transaction.Enable();
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
    ClearForUnload();
}

PLUGIN_API void XPluginReceiveMessage(XPLMPluginID, int, void*) {}
