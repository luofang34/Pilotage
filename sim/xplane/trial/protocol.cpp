#include "protocol.h"

#include <charconv>
#include <cctype>
#include <cmath>
#include <cstdlib>
#include <sstream>
#include <vector>

namespace pilotage::trial {
namespace {

std::vector<std::string> Fields(const std::string& line) {
    std::istringstream stream(line);
    std::vector<std::string> fields;
    std::string field;
    while (stream >> field) {
        fields.push_back(field);
    }
    return fields;
}

bool ParseGeneration(const std::string& value, std::uint64_t* generation) {
    if (value.empty()) {
        return false;
    }
    const char* begin = value.data();
    const char* end = begin + value.size();
    auto parsed = std::from_chars(begin, end, *generation);
    return parsed.ec == std::errc{} && parsed.ptr == end;
}

bool ParseUnsigned(const std::string& value, std::uint32_t* output) {
    if (value.empty()) {
        return false;
    }
    const char* begin = value.data();
    const char* end = begin + value.size();
    auto parsed = std::from_chars(begin, end, *output);
    return parsed.ec == std::errc{} && parsed.ptr == end;
}

bool ParseFinite(const std::string& value, double* output) {
    if (value.empty()) {
        return false;
    }
    char* end = nullptr;
    *output = std::strtod(value.c_str(), &end);
    return end == value.c_str() + value.size() && std::isfinite(*output);
}

}  // namespace

bool IsDigest(const std::string& value) {
    if (value.size() != kDigestLength) {
        return false;
    }
    for (unsigned char byte : value) {
        if (!std::isxdigit(byte) || std::isupper(byte)) {
            return false;
        }
    }
    return true;
}

ParseResult ParseCommand(const std::string& line, Command* command) {
    if (command == nullptr) {
        return ParseResult::Invalid;
    }
    const auto fields = Fields(line);
    if (fields.empty()) {
        return ParseResult::Invalid;
    }
    if (fields[0] == "CONFIG") {
        if (fields.size() != 4 ||
            !ParseGeneration(fields[1], &command->generation) ||
            !IsDigest(fields[2]) || !IsDigest(fields[3])) {
            return ParseResult::Invalid;
        }
        command->kind = CommandKind::Configure;
        command->scenario_digest = fields[2];
        command->condition_digest = fields[3];
        return ParseResult::Accepted;
    }
    if (fields[0] == "START" || fields[0] == "STOP" ||
        fields[0] == "RESET") {
        if (fields.size() != 2 ||
            !ParseGeneration(fields[1], &command->generation)) {
            return ParseResult::Invalid;
        }
        command->kind = fields[0] == "START"   ? CommandKind::Start
                        : fields[0] == "STOP" ? CommandKind::Stop
                                                : CommandKind::Reset;
        command->scenario_digest.clear();
        command->condition_digest.clear();
        return ParseResult::Accepted;
    }
    if (fields[0] == "WIND") {
        if (fields.size() != 5 ||
            !ParseGeneration(fields[1], &command->generation) ||
            !ParseUnsigned(fields[2], &command->condition_generation) ||
            !ParseFinite(fields[3], &command->wind_speed_mps) ||
            !ParseFinite(fields[4], &command->wind_direction_deg) ||
            command->wind_speed_mps < 0.0 || command->wind_speed_mps > 50.0 ||
            command->wind_direction_deg < 0.0 ||
            command->wind_direction_deg > 360.0) {
            return ParseResult::Invalid;
        }
        command->kind = CommandKind::SetWind;
        return ParseResult::Accepted;
    }
    return ParseResult::Unsupported;
}

std::string HexEncode(const std::string& value) {
    if (value.empty()) {
        return "-";
    }
    static constexpr char kHex[] = "0123456789abcdef";
    std::string result;
    result.reserve(value.size() * 2);
    for (unsigned char byte : value) {
        result.push_back(kHex[byte >> 4]);
        result.push_back(kHex[byte & 0x0F]);
    }
    return result;
}

}  // namespace pilotage::trial
