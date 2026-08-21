#include "weather_state.h"

#include <cmath>

namespace pilotage::weather {

bool IsValidRequest(const WeatherRequest& request) {
    return std::isfinite(request.wind_speed_mps) &&
           request.wind_speed_mps >= 0.0F &&
           request.wind_speed_mps <= kMaximumWindSpeedMps &&
           std::isfinite(request.wind_direction_deg) &&
           request.wind_direction_deg >= 0.0F &&
           request.wind_direction_deg <= 360.0F &&
           std::isfinite(request.turbulence) && request.turbulence >= 0.0F &&
           request.turbulence <= kMaximumTurbulence &&
           std::isfinite(request.generation) && request.generation >= 0.0F &&
           request.generation <= kMaximumGeneration &&
           std::floor(request.generation) == request.generation;
}

}  // namespace pilotage::weather
