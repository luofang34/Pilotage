#pragma once

#include <cstdint>
#include <string>

namespace pilotage::trial {

constexpr std::uint32_t kProtocolVersion = 2;
constexpr std::size_t kDigestLength = 64;

enum class CommandKind {
    Configure,
    Start,
    Stop,
    Reset,
    SetWind,
};

struct Command {
    CommandKind kind = CommandKind::Stop;
    std::uint64_t generation = 0;
    std::string scenario_digest;
    std::string condition_digest;
    std::uint32_t condition_generation = 0;
    double wind_speed_mps = 0.0;
    double wind_direction_deg = 0.0;
};

enum class ParseResult {
    Accepted,
    Invalid,
    Unsupported,
};

ParseResult ParseCommand(const std::string& line, Command* command);
bool IsDigest(const std::string& value);
std::string HexEncode(const std::string& value);

}  // namespace pilotage::trial
