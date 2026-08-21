#pragma once

namespace pilotage::weather {

constexpr float kMaximumWindSpeedMps = 50.0F;
constexpr float kMaximumTurbulence = 10.0F;
constexpr float kMaximumGeneration = 16777215.0F;

struct WeatherRequest {
    float wind_speed_mps = 0.0F;
    float wind_direction_deg = 0.0F;
    float turbulence = 0.0F;
    float generation = 0.0F;
};

enum class RequestStatus {
    Idle = 0,
    Applied = 1,
    Invalid = -1,
    DatarefMissing = -2,
};

bool IsValidRequest(const WeatherRequest& request);

}  // namespace pilotage::weather
