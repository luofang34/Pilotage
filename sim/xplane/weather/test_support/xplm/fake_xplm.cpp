#include "fake_xplm.h"

#include "XPLMProcessing.h"
#include "XPLMUtilities.h"

#include <algorithm>
#include <memory>
#include <stdexcept>
#include <unordered_map>

namespace {

struct FakeDataRef {
    std::string name;
    XPLMDataTypeID type = xplmType_Unknown;
    bool writable = false;
    int int_value = 0;
    float float_value = 0.0F;
    std::vector<float> array_value;
    fake_xplm::WriteMode write_mode = fake_xplm::WriteMode::Store;
    XPLMGetDatai_f read_int = nullptr;
    XPLMSetDatai_f write_int = nullptr;
    XPLMGetDataf_f read_float = nullptr;
    XPLMSetDataf_f write_float = nullptr;
    void* read_refcon = nullptr;
    void* write_refcon = nullptr;
};

struct FakeRegistry {
    std::unordered_map<std::string, std::unique_ptr<FakeDataRef>> refs;
    XPLMFlightLoop_f flight_loop = nullptr;
    void* flight_loop_refcon = nullptr;
    std::string debug_log;
};

FakeRegistry& Registry() {
    static FakeRegistry registry;
    return registry;
}

FakeDataRef* FromHandle(XPLMDataRef dataref) {
    return static_cast<FakeDataRef*>(dataref);
}

FakeDataRef& Require(const std::string& name) {
    const auto found = Registry().refs.find(name);
    if (found == Registry().refs.end()) {
        throw std::runtime_error("fake dataref not found: " + name);
    }
    return *found->second;
}

FakeDataRef& Add(const std::string& name, XPLMDataTypeID type,
                 bool writable) {
    auto dataref = std::make_unique<FakeDataRef>();
    dataref->name = name;
    dataref->type = type;
    dataref->writable = writable;
    FakeDataRef* value = dataref.get();
    Registry().refs[name] = std::move(dataref);
    return *value;
}

void AddInt(const char* name, int value, bool writable) {
    FakeDataRef& dataref = Add(name, xplmType_Int, writable);
    dataref.int_value = value;
}

void AddFloat(const char* name, float value, bool writable) {
    FakeDataRef& dataref = Add(name, xplmType_Float, writable);
    dataref.float_value = value;
}

void AddArray(const char* name, std::size_t count, bool writable) {
    FakeDataRef& dataref = Add(name, xplmType_FloatArray, writable);
    dataref.array_value.assign(count, 0.0F);
}

bool HasType(const FakeDataRef* dataref, XPLMDataTypeID type) {
    return dataref != nullptr && (dataref->type & type) != 0;
}

}  // namespace

namespace fake_xplm {

void Reset() { Registry() = FakeRegistry{}; }

void InstallWeatherDataRefs() {
    AddInt("sim/weather/region/update_immediately", 0, true);
    AddInt("sim/weather/region/change_mode", 0, true);
    AddFloat("sim/weather/region/variability_pct", 50.0F, true);
    AddArray("sim/weather/region/wind_speed_msc", 13, true);
    AddArray("sim/weather/region/wind_direction_degt", 13, true);
    AddArray("sim/weather/region/shear_speed_msc", 13, true);
    AddArray("sim/weather/region/shear_direction_degt", 13, true);
    AddArray("sim/weather/region/turbulence", 13, true);
    AddFloat("sim/weather/region/thermal_rate_ms", 1.0F, true);
    AddFloat("sim/weather/aircraft/wind_now_speed_msc", 0.0F, false);
    AddFloat("sim/weather/aircraft/wind_now_direction_degt", 0.0F, false);
    AddFloat("sim/weather/aircraft/wind_now_y_msc", 0.0F, false);
    AddArray("sim/weather/aircraft/turbulence", 13, false);
}

void SetType(const std::string& name, XPLMDataTypeID type) {
    Require(name).type = type;
}

void SetWritable(const std::string& name, bool writable) {
    Require(name).writable = writable;
}

void SetArraySize(const std::string& name, std::size_t size) {
    Require(name).array_value.resize(size);
}

void SetWriteMode(const std::string& name, WriteMode mode) {
    Require(name).write_mode = mode;
}

void StoreInt(const std::string& name, int value) {
    Require(name).int_value = value;
}

void StoreFloat(const std::string& name, float value) {
    Require(name).float_value = value;
}

void StoreArray(const std::string& name, const std::vector<float>& values) {
    Require(name).array_value = values;
}

int StoredInt(const std::string& name) { return Require(name).int_value; }

float StoredFloat(const std::string& name) {
    return Require(name).float_value;
}

std::vector<float> StoredArray(const std::string& name) {
    return Require(name).array_value;
}

void RunFlightLoop() {
    FakeRegistry& registry = Registry();
    if (registry.flight_loop == nullptr) {
        throw std::runtime_error("fake flight loop is not registered");
    }
    registry.flight_loop(0.0F, 0.0F, 1, registry.flight_loop_refcon);
}

}  // namespace fake_xplm

extern "C" XPLMDataRef XPLMFindDataRef(const char* name) {
    if (name == nullptr) {
        return nullptr;
    }
    const auto found = Registry().refs.find(name);
    return found == Registry().refs.end() ? nullptr : found->second.get();
}

extern "C" int XPLMCanWriteDataRef(XPLMDataRef dataref) {
    const FakeDataRef* value = FromHandle(dataref);
    return value != nullptr && value->writable ? 1 : 0;
}

extern "C" XPLMDataTypeID XPLMGetDataRefTypes(XPLMDataRef dataref) {
    const FakeDataRef* value = FromHandle(dataref);
    return value == nullptr ? xplmType_Unknown : value->type;
}

extern "C" int XPLMGetDatai(XPLMDataRef dataref) {
    FakeDataRef* value = FromHandle(dataref);
    if (!HasType(value, xplmType_Int)) {
        return 0;
    }
    return value->read_int == nullptr ? value->int_value
                                      : value->read_int(value->read_refcon);
}

extern "C" void XPLMSetDatai(XPLMDataRef dataref, int input) {
    FakeDataRef* value = FromHandle(dataref);
    if (!HasType(value, xplmType_Int) || !value->writable ||
        value->write_mode == fake_xplm::WriteMode::Ignore) {
        return;
    }
    if (value->write_int == nullptr) {
        value->int_value = input;
    } else {
        value->write_int(value->write_refcon, input);
    }
}

extern "C" float XPLMGetDataf(XPLMDataRef dataref) {
    FakeDataRef* value = FromHandle(dataref);
    if (!HasType(value, xplmType_Float)) {
        return 0.0F;
    }
    return value->read_float == nullptr
               ? value->float_value
               : value->read_float(value->read_refcon);
}

extern "C" void XPLMSetDataf(XPLMDataRef dataref, float input) {
    FakeDataRef* value = FromHandle(dataref);
    if (!HasType(value, xplmType_Float) || !value->writable ||
        value->write_mode == fake_xplm::WriteMode::Ignore) {
        return;
    }
    if (value->write_float == nullptr) {
        value->float_value = input;
    } else {
        value->write_float(value->write_refcon, input);
    }
}

extern "C" int XPLMGetDatavf(XPLMDataRef dataref, float* output, int offset,
                               int maximum) {
    FakeDataRef* value = FromHandle(dataref);
    if (!HasType(value, xplmType_FloatArray)) {
        return 0;
    }
    if (output == nullptr) {
        return static_cast<int>(value->array_value.size());
    }
    if (offset < 0 || maximum <= 0 ||
        static_cast<std::size_t>(offset) >= value->array_value.size()) {
        return 0;
    }
    const std::size_t available = value->array_value.size() - offset;
    const std::size_t count =
        std::min(available, static_cast<std::size_t>(maximum));
    std::copy_n(value->array_value.begin() + offset, count, output);
    return static_cast<int>(count);
}

extern "C" void XPLMSetDatavf(XPLMDataRef dataref, float* input, int offset,
                                int count) {
    FakeDataRef* value = FromHandle(dataref);
    if (!HasType(value, xplmType_FloatArray) || !value->writable ||
        value->write_mode == fake_xplm::WriteMode::Ignore || input == nullptr ||
        offset < 0 || count <= 0 ||
        static_cast<std::size_t>(offset) >= value->array_value.size()) {
        return;
    }
    const std::size_t available = value->array_value.size() - offset;
    const std::size_t copied =
        std::min(available, static_cast<std::size_t>(count));
    std::copy_n(input, copied, value->array_value.begin() + offset);
}

extern "C" XPLMDataRef XPLMRegisterDataAccessor(
    const char* name, XPLMDataTypeID type, int writable,
    XPLMGetDatai_f read_int, XPLMSetDatai_f write_int,
    XPLMGetDataf_f read_float, XPLMSetDataf_f write_float, XPLMGetDatad_f,
    XPLMSetDatad_f, XPLMGetDatavi_f, XPLMSetDatavi_f, XPLMGetDatavf_f,
    XPLMSetDatavf_f, XPLMGetDatab_f, XPLMSetDatab_f, void* read_refcon,
    void* write_refcon) {
    FakeDataRef& dataref = Add(name, type, writable != 0);
    dataref.read_int = read_int;
    dataref.write_int = write_int;
    dataref.read_float = read_float;
    dataref.write_float = write_float;
    dataref.read_refcon = read_refcon;
    dataref.write_refcon = write_refcon;
    return &dataref;
}

extern "C" void XPLMUnregisterDataAccessor(XPLMDataRef dataref) {
    for (auto iterator = Registry().refs.begin();
         iterator != Registry().refs.end(); ++iterator) {
        if (iterator->second.get() == dataref) {
            Registry().refs.erase(iterator);
            return;
        }
    }
}

extern "C" void XPLMRegisterFlightLoopCallback(
    XPLMFlightLoop_f callback, float, void* refcon) {
    Registry().flight_loop = callback;
    Registry().flight_loop_refcon = refcon;
}

extern "C" void XPLMUnregisterFlightLoopCallback(
    XPLMFlightLoop_f callback, void* refcon) {
    if (Registry().flight_loop == callback &&
        Registry().flight_loop_refcon == refcon) {
        Registry().flight_loop = nullptr;
        Registry().flight_loop_refcon = nullptr;
    }
}

extern "C" void XPLMDebugString(const char* message) {
    if (message != nullptr) {
        Registry().debug_log += message;
    }
}
