#pragma once

#include "XPLMDataAccess.h"

#include <cstddef>
#include <string>
#include <vector>

namespace fake_xplm {

enum class WriteMode {
    Store,
    Ignore,
};

void Reset();
void InstallWeatherDataRefs();
void SetType(const std::string& name, XPLMDataTypeID type);
void SetWritable(const std::string& name, bool writable);
void SetArraySize(const std::string& name, std::size_t size);
void SetWriteMode(const std::string& name, WriteMode mode);
void StoreInt(const std::string& name, int value);
void StoreFloat(const std::string& name, float value);
void StoreArray(const std::string& name, const std::vector<float>& values);
int StoredInt(const std::string& name);
float StoredFloat(const std::string& name);
std::vector<float> StoredArray(const std::string& name);
void RunFlightLoop();

}  // namespace fake_xplm
