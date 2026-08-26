#include "protocol.h"

#include <cassert>
#include <string>

using pilotage::trial::Command;
using pilotage::trial::CommandKind;
using pilotage::trial::HexEncode;
using pilotage::trial::IsDigest;
using pilotage::trial::ParseCommand;
using pilotage::trial::ParseResult;

int main() {
    const std::string first(64, '1');
    const std::string second(64, 'a');
    Command command;
    assert(ParseCommand("CONFIG 7 " + first + " " + second, &command) ==
           ParseResult::Accepted);
    assert(command.kind == CommandKind::Configure);
    assert(command.generation == 7);
    assert(command.scenario_digest == first);
    assert(command.condition_digest == second);

    assert(ParseCommand("START 8", &command) == ParseResult::Accepted);
    assert(command.kind == CommandKind::Start);
    assert(command.generation == 8);
    assert(ParseCommand("STOP 8", &command) == ParseResult::Accepted);
    assert(command.kind == CommandKind::Stop);
    assert(ParseCommand("RESET 9", &command) == ParseResult::Accepted);
    assert(command.kind == CommandKind::Reset);
    assert(ParseCommand("WIND 9 4 12.5 270", &command) ==
           ParseResult::Accepted);
    assert(command.kind == CommandKind::SetWind);
    assert(command.condition_generation == 4);
    assert(command.wind_speed_mps == 12.5);
    assert(command.wind_direction_deg == 270.0);

    assert(ParseCommand("START", &command) == ParseResult::Invalid);
    assert(ParseCommand("START -1", &command) == ParseResult::Invalid);
    assert(ParseCommand("CONFIG 1 bad " + second, &command) ==
           ParseResult::Invalid);
    assert(ParseCommand("RUN 1", &command) == ParseResult::Unsupported);
    assert(ParseCommand("WIND 1 2 nan 0", &command) == ParseResult::Invalid);
    assert(ParseCommand("WIND 1 2 51 0", &command) == ParseResult::Invalid);
    assert(!IsDigest(std::string(64, 'A')));
    assert(!IsDigest(std::string(63, 'a')));
    assert(HexEncode("A z") == "41207a");
    assert(HexEncode("") == "-");
}
