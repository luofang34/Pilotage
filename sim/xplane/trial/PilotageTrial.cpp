#include "XPLMDataAccess.h"
#include "XPLMPlanes.h"
#include "XPLMPlugin.h"
#include "XPLMProcessing.h"
#include "XPLMUtilities.h"

#include "link.h"
#include "protocol.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <iomanip>
#include <limits>
#include <sstream>
#include <string>

#ifndef PILOTAGE_TRIAL_BUILD_ID
#define PILOTAGE_TRIAL_BUILD_ID "unidentified"
#endif

#ifndef PILOTAGE_BRIDGE_BUILD_DIGEST
#define PILOTAGE_BRIDGE_BUILD_DIGEST "unidentified"
#endif

namespace {

using pilotage::trial::Command;
using pilotage::trial::CommandKind;
using pilotage::trial::HexEncode;
using pilotage::trial::Link;
using pilotage::trial::ParseCommand;
using pilotage::trial::ParseResult;

struct Datarefs {
    XPLMDataRef sim_time = nullptr;
    XPLMDataRef local_x = nullptr;
    XPLMDataRef local_y = nullptr;
    XPLMDataRef local_z = nullptr;
    XPLMDataRef local_vx = nullptr;
    XPLMDataRef local_vy = nullptr;
    XPLMDataRef local_vz = nullptr;
    XPLMDataRef local_ax = nullptr;
    XPLMDataRef local_ay = nullptr;
    XPLMDataRef local_az = nullptr;
    XPLMDataRef g_axial = nullptr;
    XPLMDataRef g_side = nullptr;
    XPLMDataRef g_normal = nullptr;
    XPLMDataRef quaternion = nullptr;
    XPLMDataRef roll_rate = nullptr;
    XPLMDataRef pitch_rate = nullptr;
    XPLMDataRef yaw_rate = nullptr;
    XPLMDataRef on_ground = nullptr;
    XPLMDataRef crashed = nullptr;
    XPLMDataRef wind_speed = nullptr;
    XPLMDataRef wind_direction = nullptr;
    XPLMDataRef requested_wind_speed = nullptr;
    XPLMDataRef requested_wind_direction = nullptr;
    XPLMDataRef requested_turbulence = nullptr;
    XPLMDataRef apply_weather_generation = nullptr;
    XPLMDataRef applied_weather_generation = nullptr;
    XPLMDataRef weather_status = nullptr;
};

struct TrialState {
    std::uint64_t configured_generation = 0;
    std::uint64_t active_generation = 0;
    std::uint64_t reset_generation = 0;
    std::uint64_t pending_reset_generation = 0;
    std::uint64_t sequence = 0;
    std::string scenario_digest;
    std::string condition_digest;
    std::string aircraft_path;
    double prior_sim_time = 0.0;
    double start_sim_time = 0.0;
    bool active = false;
    bool link_was_up = false;
    bool wind_pending = false;
    std::uint32_t condition_generation = 0;
    std::uint64_t wind_trial_generation = 0;
};

Datarefs refs;
TrialState state;
XPLMCommandRef reset_command = nullptr;
bool loop_registered = false;

XPLMDataRef Find(const char* name) {
    return XPLMFindDataRef(name);
}

void BindDatarefs() {
    refs.sim_time = Find("sim/time/total_flight_time_sec");
    if (refs.sim_time == nullptr) {
        // Everything this plugin observes is gated on a finite simulator
        // clock, so without this one dataref it connects, says hello, and then
        // goes mute forever — sending no samples and noticing no aircraft
        // change. It fails safe, but silently: the host reaches its timeout
        // with nothing saying why. Say why.
        XPLMDebugString(
            "PilotageTrial: sim/time/total_flight_time_sec did not bind; "
            "no samples will be sent\n");
    }
    refs.local_x = Find("sim/flightmodel/position/local_x");
    refs.local_y = Find("sim/flightmodel/position/local_y");
    refs.local_z = Find("sim/flightmodel/position/local_z");
    refs.local_vx = Find("sim/flightmodel/position/local_vx");
    refs.local_vy = Find("sim/flightmodel/position/local_vy");
    refs.local_vz = Find("sim/flightmodel/position/local_vz");
    refs.local_ax = Find("sim/flightmodel/position/local_ax");
    refs.local_ay = Find("sim/flightmodel/position/local_ay");
    refs.local_az = Find("sim/flightmodel/position/local_az");
    refs.g_axial = Find("sim/flightmodel/forces/g_axil");
    refs.g_side = Find("sim/flightmodel/forces/g_side");
    refs.g_normal = Find("sim/flightmodel/forces/g_nrml");
    refs.quaternion = Find("sim/flightmodel/position/q");
    refs.roll_rate = Find("sim/flightmodel/position/Prad");
    refs.pitch_rate = Find("sim/flightmodel/position/Qrad");
    refs.yaw_rate = Find("sim/flightmodel/position/Rrad");
    refs.on_ground = Find("sim/flightmodel/failures/onground_any");
    refs.crashed = Find("sim/flightmodel2/misc/has_crashed");
    refs.wind_speed = Find("sim/weather/aircraft/wind_now_speed_msc");
    refs.wind_direction =
        Find("sim/weather/aircraft/wind_now_direction_degt");
    refs.requested_wind_speed = Find("pilotage/weather/wind_speed_mps");
    refs.requested_wind_direction =
        Find("pilotage/weather/wind_direction_deg");
    refs.requested_turbulence = Find("pilotage/weather/turbulence");
    refs.apply_weather_generation =
        Find("pilotage/weather/apply_generation");
    refs.applied_weather_generation =
        Find("pilotage/weather/applied_generation");
    refs.weather_status = Find("pilotage/weather/status");
    reset_command = XPLMFindCommand("sim/operation/reset_flight");
}

std::string AircraftPath() {
    char name[256]{};
    char path[1024]{};
    XPLMGetNthAircraftModel(0, name, path);
    return path;
}

std::string PluginPath(XPLMPluginID id) {
    char name[256]{};
    char path[1024]{};
    char signature[256]{};
    char description[256]{};
    XPLMGetPluginInfo(id, name, path, signature, description);
    return path;
}

std::string BridgePath() {
    const int count = XPLMCountPlugins();
    for (int index = 0; index < count; ++index) {
        const XPLMPluginID id = XPLMGetNthPlugin(index);
        char name[256]{};
        char path[1024]{};
        char signature[256]{};
        char description[256]{};
        XPLMGetPluginInfo(id, name, path, signature, description);
        std::string searchable = std::string(name) + " " + signature + " " + path;
        std::transform(searchable.begin(), searchable.end(), searchable.begin(),
                       [](unsigned char byte) { return std::tolower(byte); });
        if (searchable.find("px4xplane") != std::string::npos) {
            return path;
        }
    }
    return {};
}

std::string HelloLine() {
    int xplane_version = 0;
    int sdk_version = 0;
    XPLMHostApplicationID host = 0;
    XPLMGetVersions(&xplane_version, &sdk_version, &host);
    std::ostringstream line;
    line << "HELLO " << pilotage::trial::kProtocolVersion << ' '
         << xplane_version << ' ' << sdk_version << ' ' << host << ' '
         << HexEncode(PILOTAGE_TRIAL_BUILD_ID) << ' '
         << PILOTAGE_BRIDGE_BUILD_DIGEST << ' '
         << HexEncode(state.aircraft_path) << ' '
         << HexEncode(PluginPath(XPLMGetMyID())) << ' '
         << HexEncode(BridgePath());
    return line.str();
}

void SendIdentity() {
    state.aircraft_path = AircraftPath();
    Link().SendReply(HelloLine());
    if (state.active) {
        std::ostringstream line;
        line << "ACTIVE " << state.active_generation << ' '
             << state.scenario_digest << ' ' << state.condition_digest << ' '
             << state.reset_generation;
        Link().SendReply(line.str());
    }
}

void Reject(const char* code, std::uint64_t generation) {
    std::ostringstream line;
    line << "ERROR " << generation << ' ' << code;
    Link().SendReply(line.str());
}

void Configure(const Command& command) {
    if (state.active || state.pending_reset_generation != 0 ||
        command.generation == 0 ||
        command.generation <= state.configured_generation) {
        Reject("invalid_generation", command.generation);
        return;
    }
    state.configured_generation = command.generation;
    state.scenario_digest = command.scenario_digest;
    state.condition_digest = command.condition_digest;
    std::ostringstream line;
    line << "CONFIGURED " << command.generation << ' '
         << state.scenario_digest << ' ' << state.condition_digest;
    Link().SendReply(line.str());
}

void Start(const Command& command, double sim_time) {
    if (state.active || state.pending_reset_generation != 0 ||
        command.generation == 0 ||
        command.generation != state.configured_generation) {
        Reject("not_configured", command.generation);
        return;
    }
    state.active = true;
    state.active_generation = command.generation;
    state.start_sim_time = sim_time;
    state.sequence = 0;
    std::ostringstream line;
    line << "STARTED " << command.generation << ' ' << std::setprecision(17)
         << sim_time << ' ' << state.reset_generation;
    Link().SendReply(line.str());
}

void Stop(const Command& command, double sim_time) {
    if (!state.active || command.generation != state.active_generation) {
        Reject("not_active", command.generation);
        return;
    }
    state.active = false;
    std::ostringstream line;
    line << "STOPPED " << command.generation << ' ' << state.sequence << ' '
         << std::setprecision(17) << sim_time;
    Link().SendReply(line.str());
}

void Reset(const Command& command) {
    if (command.generation == 0 || command.generation < state.configured_generation ||
        state.pending_reset_generation != 0 || reset_command == nullptr) {
        Reject("reset_refused", command.generation);
        return;
    }
    state.active = false;
    state.wind_pending = false;
    state.pending_reset_generation = command.generation;
    Link().SendReply("RESETTING " + std::to_string(command.generation));
    XPLMCommandOnce(reset_command);
}

bool WeatherControlAvailable() {
    return refs.requested_wind_speed != nullptr &&
           refs.requested_wind_direction != nullptr &&
           refs.requested_turbulence != nullptr &&
           refs.apply_weather_generation != nullptr &&
           refs.applied_weather_generation != nullptr &&
           refs.weather_status != nullptr && refs.wind_speed != nullptr &&
           refs.wind_direction != nullptr;
}

void SetWind(const Command& command) {
    if (command.generation == 0 ||
        command.generation != state.configured_generation ||
        command.condition_generation == 0 ||
        command.condition_generation > 16777215 || state.wind_pending ||
        !WeatherControlAvailable()) {
        Reject("wind_refused", command.generation);
        return;
    }
    XPLMSetDataf(refs.requested_wind_speed,
                 static_cast<float>(command.wind_speed_mps));
    XPLMSetDataf(refs.requested_wind_direction,
                 static_cast<float>(command.wind_direction_deg));
    XPLMSetDataf(refs.requested_turbulence, 0.0F);
    XPLMSetDataf(refs.apply_weather_generation,
                 static_cast<float>(command.condition_generation));
    state.wind_pending = true;
    state.condition_generation = command.condition_generation;
    state.wind_trial_generation = command.generation;
}

void ObserveWind() {
    if (!state.wind_pending) {
        return;
    }
    const float applied = XPLMGetDataf(refs.applied_weather_generation);
    const float status = XPLMGetDataf(refs.weather_status);
    if (status < 0.0F) {
        Reject("wind_apply_failed", state.wind_trial_generation);
        state.wind_pending = false;
        return;
    }
    if (applied != static_cast<float>(state.condition_generation) ||
        status != 1.0F) {
        return;
    }
    std::ostringstream line;
    line << std::setprecision(17) << "WIND_APPLIED "
         << state.wind_trial_generation << ' ' << state.condition_generation
         << ' ' << static_cast<double>(XPLMGetDataf(refs.wind_speed)) << ' '
         << static_cast<double>(XPLMGetDataf(refs.wind_direction));
    Link().SendReply(line.str());
    state.wind_pending = false;
}

void ProcessCommands(double sim_time) {
    std::string line;
    while (Link().TakeLine(&line)) {
        Command command;
        const ParseResult result = ParseCommand(line, &command);
        if (result != ParseResult::Accepted) {
            Reject(result == ParseResult::Unsupported ? "unsupported" : "invalid", 0);
            continue;
        }
        switch (command.kind) {
            case CommandKind::Configure:
                Configure(command);
                break;
            case CommandKind::Start:
                Start(command, sim_time);
                break;
            case CommandKind::Stop:
                Stop(command, sim_time);
                break;
            case CommandKind::Reset:
                Reset(command);
                break;
            case CommandKind::SetWind:
                SetWind(command);
                break;
        }
    }
}

double ReadDouble(XPLMDataRef ref) {
    return ref == nullptr ? std::numeric_limits<double>::quiet_NaN()
                          : XPLMGetDatad(ref);
}

double ReadFloat(XPLMDataRef ref) {
    return ref == nullptr ? std::numeric_limits<double>::quiet_NaN()
                          : static_cast<double>(XPLMGetDataf(ref));
}

int ReadInt(XPLMDataRef ref) {
    return ref == nullptr ? -1 : XPLMGetDatai(ref);
}

std::array<float, 4> ReadQuaternion() {
    std::array<float, 4> value{};
    if (refs.quaternion == nullptr ||
        XPLMGetDatavf(refs.quaternion, value.data(), 0, 4) != 4) {
        value.fill(std::numeric_limits<float>::quiet_NaN());
    }
    return value;
}

void SendSample(double sim_time) {
    if (!state.active) {
        return;
    }
    const auto quaternion = ReadQuaternion();
    std::ostringstream line;
    line << std::setprecision(17) << "SAMPLE " << state.active_generation << ' '
         << state.sequence << ' ' << sim_time << ' '
         << (sim_time - state.start_sim_time) << ' ' << state.reset_generation
         << ' ' << ReadDouble(refs.local_x) << ' ' << ReadDouble(refs.local_y)
         << ' ' << ReadDouble(refs.local_z) << ' ' << ReadFloat(refs.local_vx)
         << ' ' << ReadFloat(refs.local_vy) << ' ' << ReadFloat(refs.local_vz)
         << ' ' << ReadFloat(refs.local_ax) << ' ' << ReadFloat(refs.local_ay)
         << ' ' << ReadFloat(refs.local_az) << ' ' << ReadFloat(refs.g_axial)
         << ' ' << ReadFloat(refs.g_side) << ' ' << ReadFloat(refs.g_normal)
         << ' ' << quaternion[0] << ' ' << quaternion[1] << ' '
         << quaternion[2] << ' ' << quaternion[3]
         << ' ' << ReadFloat(refs.roll_rate) << ' '
         << ReadFloat(refs.pitch_rate) << ' ' << ReadFloat(refs.yaw_rate) << ' '
         << ReadInt(refs.on_ground) << ' ' << ReadInt(refs.crashed) << ' '
         << ReadFloat(refs.wind_speed) << ' ' << ReadFloat(refs.wind_direction);
    Link().SendSample(line.str());
    state.sequence += 1;
}

void ObserveEpoch(double sim_time) {
    if (sim_time + 1.0e-6 < state.prior_sim_time) {
        state.reset_generation += 1;
        state.active = false;
        if (state.pending_reset_generation != 0) {
            std::ostringstream line;
            line << std::setprecision(17) << "RESET_COMPLETE "
                 << state.pending_reset_generation << ' '
                 << state.reset_generation << ' ' << sim_time;
            Link().SendReply(line.str());
            state.pending_reset_generation = 0;
        } else {
            Link().SendReply("REWIND " +
                             std::to_string(state.reset_generation));
        }
    }
    state.prior_sim_time = sim_time;
    const std::string current_aircraft = AircraftPath();
    if (current_aircraft != state.aircraft_path) {
        state.active = false;
        state.aircraft_path = current_aircraft;
        Link().SendReply("AIRCRAFT_CHANGED");
        Link().SendReply(HelloLine());
    }
}

float FlightLoop(float, float, int, void*) {
    Link().Pump();
    if (Link().connected() && !state.link_was_up) {
        SendIdentity();
    }
    state.link_was_up = Link().connected();
    const double sim_time = ReadFloat(refs.sim_time);
    if (std::isfinite(sim_time)) {
        ObserveEpoch(sim_time);
        ProcessCommands(sim_time);
        ObserveWind();
        SendSample(sim_time);
    }
    state.link_was_up = Link().connected();
    return -1.0F;
}

}  // namespace

PLUGIN_API int XPluginStart(char* out_name, char* out_sig, char* out_desc) {
    std::strcpy(out_name, "PilotageTrial");
    std::strcpy(out_sig, "systems.sokoly.pilotage.trial");
    std::strcpy(out_desc, "Verified X-Plane trial observation link");
    return 1;
}

PLUGIN_API void XPluginStop(void) {
    if (loop_registered) {
        XPLMUnregisterFlightLoopCallback(FlightLoop, nullptr);
        loop_registered = false;
    }
    Link().Close();
}

PLUGIN_API int XPluginEnable(void) {
    BindDatarefs();
    state.aircraft_path = AircraftPath();
    state.prior_sim_time = ReadFloat(refs.sim_time);
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
    Link().Close();
}

PLUGIN_API void XPluginReceiveMessage(XPLMPluginID, int, void*) {}
