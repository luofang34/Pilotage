#include "XPLMDataAccess.h"
#include "XPLMPlugin.h"
#include "fake_xplm.h"

#include <array>
#include <cassert>
#include <cmath>
#include <functional>
#include <limits>
#include <vector>

PLUGIN_API int XPluginStart(char* out_name, char* out_sig, char* out_desc);
PLUGIN_API void XPluginStop(void);
PLUGIN_API int XPluginEnable(void);

namespace {

constexpr const char* kUpdate =
    "sim/weather/region/update_immediately";
constexpr const char* kChangeMode = "sim/weather/region/change_mode";
constexpr const char* kVariability = "sim/weather/region/variability_pct";
constexpr const char* kWindSpeed = "sim/weather/region/wind_speed_msc";
constexpr const char* kWindDirection =
    "sim/weather/region/wind_direction_degt";
constexpr const char* kShearSpeed = "sim/weather/region/shear_speed_msc";
constexpr const char* kShearDirection =
    "sim/weather/region/shear_direction_degt";
constexpr const char* kTurbulence = "sim/weather/region/turbulence";
constexpr const char* kThermalRate = "sim/weather/region/thermal_rate_ms";
constexpr const char* kActualTurbulence =
    "sim/weather/aircraft/turbulence";
constexpr const char* kStagedSpeed = "pilotage/weather/wind_speed_mps";
constexpr const char* kStagedDirection =
    "pilotage/weather/wind_direction_deg";
constexpr const char* kStagedTurbulence =
    "pilotage/weather/turbulence_scale";
constexpr const char* kClearGeneration =
    "pilotage/weather/clear_generation";
constexpr const char* kApplyGeneration =
    "pilotage/weather/apply_generation";
constexpr const char* kStatus = "pilotage/weather/status";
constexpr const char* kResponseGeneration =
    "pilotage/weather/response_generation";
constexpr const char* kResponseOperation =
    "pilotage/weather/response_operation";
constexpr const char* kAppliedGeneration =
    "pilotage/weather/applied_generation";
constexpr const char* kResponseSpeed =
    "pilotage/weather/response_wind_speed_mps";
constexpr const char* kResponseDirection =
    "pilotage/weather/response_wind_direction_deg";
constexpr const char* kResponseTurbulence =
    "pilotage/weather/response_turbulence_scale";

const std::array<const char*, 5> kRegionalArrays = {
    kWindSpeed, kWindDirection, kShearSpeed, kShearDirection, kTurbulence};

class PluginSession {
  public:
    PluginSession() {
        std::array<char, 256> name{};
        std::array<char, 256> signature{};
        std::array<char, 256> description{};
        assert(XPluginStart(name.data(), signature.data(),
                            description.data()) == 1);
        assert(XPluginEnable() == 1);
    }

    ~PluginSession() { XPluginStop(); }

    PluginSession(const PluginSession&) = delete;
    PluginSession& operator=(const PluginSession&) = delete;
};

XPLMDataRef Ref(const char* name) {
    XPLMDataRef dataref = XPLMFindDataRef(name);
    assert(dataref != nullptr);
    return dataref;
}

float Read(const char* name) { return XPLMGetDataf(Ref(name)); }

void Write(const char* name, float value) {
    XPLMSetDataf(Ref(name), value);
}

std::vector<float> Filled(float value) {
    return std::vector<float>(13, value);
}

void PrepareSimulator() {
    fake_xplm::Reset();
    fake_xplm::InstallWeatherDataRefs();
}

void FillRegion(float value) {
    for (const char* name : kRegionalArrays) {
        fake_xplm::StoreArray(name, Filled(value));
    }
    fake_xplm::StoreInt(kUpdate, 0);
    fake_xplm::StoreInt(kChangeMode, 0);
    fake_xplm::StoreFloat(kVariability, value);
    fake_xplm::StoreFloat(kThermalRate, value);
}

void TriggerClear() {
    Write(kClearGeneration, 1.0F);
    fake_xplm::RunFlightLoop();
}

void AssertClearFailure(float expected_status) {
    assert(Read(kStatus) == expected_status);
    assert(Read(kStatus) != 3.0F);
    assert(Read(kResponseGeneration) == 1.0F);
    assert(Read(kResponseOperation) == 2.0F);
    assert(Read(kAppliedGeneration) == 0.0F);
}

void CheckClearRefusal(const std::function<void()>& configure,
                       float expected_status) {
    PrepareSimulator();
    configure();
    PluginSession plugin;
    TriggerClear();
    AssertClearFailure(expected_status);
}

void AssertArrayEquals(const char* name, float expected) {
    const std::vector<float> values = fake_xplm::StoredArray(name);
    assert(values.size() == 13);
    for (float value : values) {
        assert(value == expected);
    }
}

void TestRegionTypeAndWritableRefusal() {
    CheckClearRefusal(
        [] { fake_xplm::SetType(kUpdate, xplmType_Float); }, -2.0F);
    CheckClearRefusal(
        [] { fake_xplm::SetWritable(kChangeMode, false); }, -2.0F);
    CheckClearRefusal(
        [] { fake_xplm::SetType(kVariability, xplmType_Int); }, -2.0F);
    CheckClearRefusal(
        [] { fake_xplm::SetWritable(kThermalRate, false); }, -2.0F);
    CheckClearRefusal(
        [] { fake_xplm::SetType(kWindSpeed, xplmType_Float); }, -2.0F);
    CheckClearRefusal(
        [] { fake_xplm::SetWritable(kWindSpeed, false); }, -2.0F);
}

void TestEveryRegionArrayRequiresExactlyThirteenLayers() {
    for (const char* name : kRegionalArrays) {
        CheckClearRefusal(
            [name] { fake_xplm::SetArraySize(name, 12); }, -2.0F);
    }
}

void TestNoOpClearDoesNotAcknowledge() {
    PrepareSimulator();
    FillRegion(9.0F);
    fake_xplm::SetWriteMode(kWindSpeed, fake_xplm::WriteMode::Ignore);
    PluginSession plugin;
    TriggerClear();
    AssertClearFailure(-5.0F);
    AssertArrayEquals(kWindSpeed, 9.0F);
}

void TestSuccessfulClearWritesCanonicalCalm() {
    PrepareSimulator();
    FillRegion(9.0F);
    PluginSession plugin;
    Write(kStagedSpeed, 12.0F);
    Write(kStagedDirection, 275.0F);
    Write(kStagedTurbulence, 4.0F);
    TriggerClear();

    assert(Read(kStatus) == 3.0F);
    assert(Read(kAppliedGeneration) == 1.0F);
    assert(Read(kResponseGeneration) == 1.0F);
    assert(Read(kResponseOperation) == 2.0F);
    assert(Read(kResponseSpeed) == 0.0F);
    assert(Read(kResponseDirection) == 0.0F);
    assert(Read(kResponseTurbulence) == 0.0F);
    assert(Read(kStagedSpeed) == 0.0F);
    assert(Read(kStagedDirection) == 0.0F);
    assert(Read(kStagedTurbulence) == 0.0F);
    for (const char* name : kRegionalArrays) {
        AssertArrayEquals(name, 0.0F);
    }
    assert(fake_xplm::StoredInt(kUpdate) == 1);
    assert(fake_xplm::StoredInt(kChangeMode) == 3);
    assert(fake_xplm::StoredFloat(kVariability) == 0.0F);
    assert(fake_xplm::StoredFloat(kThermalRate) == 0.0F);

    Write(kApplyGeneration, 2.0F);
    fake_xplm::RunFlightLoop();
    assert(Read(kStatus) == 1.0F);
    assert(Read(kAppliedGeneration) == 2.0F);
    for (const char* name : kRegionalArrays) {
        AssertArrayEquals(name, 0.0F);
    }
}

void TestUpdateImmediatelyNoOpRefusesReadback() {
    PrepareSimulator();
    FillRegion(9.0F);
    fake_xplm::SetWriteMode(kUpdate, fake_xplm::WriteMode::Ignore);
    PluginSession plugin;
    TriggerClear();
    AssertClearFailure(-5.0F);
    assert(fake_xplm::StoredInt(kUpdate) == 0);
}

void TestApplyUsesSnapshotAndStopClearsRegion() {
    PrepareSimulator();
    {
        PluginSession plugin;
        Write(kStagedSpeed, 5.0F);
        Write(kStagedDirection, 270.0F);
        Write(kStagedTurbulence, 2.0F);
        Write(kApplyGeneration, 1.0F);
        Write(kStagedSpeed, 9.0F);
        Write(kStagedDirection, 90.0F);
        Write(kStagedTurbulence, 4.0F);
        fake_xplm::RunFlightLoop();

        assert(Read(kStatus) == 1.0F);
        assert(Read(kResponseSpeed) == 5.0F);
        assert(Read(kResponseDirection) == 270.0F);
        assert(Read(kResponseTurbulence) == 2.0F);
        AssertArrayEquals(kWindSpeed, 5.0F);
        AssertArrayEquals(kWindDirection, 270.0F);
        AssertArrayEquals(kTurbulence, 2.0F);
        assert(Read(kStagedSpeed) == 9.0F);
        assert(Read(kStagedDirection) == 90.0F);
        assert(Read(kStagedTurbulence) == 4.0F);
    }
    for (const char* name : kRegionalArrays) {
        AssertArrayEquals(name, 0.0F);
    }
}

void TestAircraftTurbulenceRequiresThirteenLayers() {
    PrepareSimulator();
    FillRegion(9.0F);
    fake_xplm::SetArraySize(kActualTurbulence, 12);
    PluginSession plugin;
    Write(kStagedSpeed, 12.0F);
    Write(kStagedDirection, 275.0F);
    Write(kStagedTurbulence, 4.0F);
    TriggerClear();
    AssertClearFailure(-2.0F);
    for (const char* name : kRegionalArrays) {
        AssertArrayEquals(name, 0.0F);
    }
    assert(Read(kStagedSpeed) == 0.0F);
    assert(Read(kStagedDirection) == 0.0F);
    assert(Read(kStagedTurbulence) == 0.0F);
}

void TestProtocolVersionIsReadOnly() {
    PrepareSimulator();
    PluginSession plugin;
    XPLMDataRef version = Ref("pilotage/weather/protocol_version");
    assert((XPLMGetDataRefTypes(version) & xplmType_Float) != 0);
    assert(XPLMCanWriteDataRef(version) == 0);
    assert(XPLMGetDataf(version) == 1.0F);
    XPLMSetDataf(version, 9.0F);
    assert(XPLMGetDataf(version) == 1.0F);
}

void TestActualTurbulencePublishesMaximumLayer() {
    PrepareSimulator();
    fake_xplm::StoreArray(
        kActualTurbulence,
        {0.1F, 0.4F, 2.0F, 0.2F, 1.0F, 0.3F, 0.5F,
         0.7F, 0.6F, 0.8F, 0.9F, 1.1F, 0.0F});
    PluginSession plugin;
    fake_xplm::RunFlightLoop();
    assert(Read("pilotage/weather/actual_turbulence_profile_max") == 2.0F);

    std::vector<float> invalid = Filled(0.0F);
    invalid[4] = std::numeric_limits<float>::quiet_NaN();
    fake_xplm::StoreArray(kActualTurbulence, invalid);
    fake_xplm::RunFlightLoop();
    assert(std::isnan(
        Read("pilotage/weather/actual_turbulence_profile_max")));

    fake_xplm::SetArraySize(kActualTurbulence, 12);
    fake_xplm::RunFlightLoop();
    assert(std::isnan(
        Read("pilotage/weather/actual_turbulence_profile_max")));

    for (float invalid_value : {-0.1F, 10.1F}) {
        invalid = Filled(0.0F);
        invalid[4] = invalid_value;
        fake_xplm::StoreArray(kActualTurbulence, invalid);
        fake_xplm::RunFlightLoop();
        assert(std::isnan(
            Read("pilotage/weather/actual_turbulence_profile_max")));
    }
}

}  // namespace

int main() {
    TestRegionTypeAndWritableRefusal();
    TestEveryRegionArrayRequiresExactlyThirteenLayers();
    TestNoOpClearDoesNotAcknowledge();
    TestSuccessfulClearWritesCanonicalCalm();
    TestUpdateImmediatelyNoOpRefusesReadback();
    TestApplyUsesSnapshotAndStopClearsRegion();
    TestAircraftTurbulenceRequiresThirteenLayers();
    TestProtocolVersionIsReadOnly();
    TestActualTurbulencePublishesMaximumLayer();
    return 0;
}
