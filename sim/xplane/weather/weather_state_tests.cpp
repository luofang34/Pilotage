#include "weather_state.h"

#include <cassert>
#include <limits>

using pilotage::weather::IsValidRequest;
using pilotage::weather::WeatherRequest;

int main() {
    assert(IsValidRequest(WeatherRequest{5.0F, 270.0F, 0.2F, 1.0F}));
    assert(IsValidRequest(WeatherRequest{0.0F, 360.0F, 0.0F, 0.0F}));
    assert(!IsValidRequest(WeatherRequest{-1.0F, 0.0F, 0.0F, 1.0F}));
    assert(!IsValidRequest(WeatherRequest{51.0F, 0.0F, 0.0F, 1.0F}));
    assert(!IsValidRequest(WeatherRequest{1.0F, 361.0F, 0.0F, 1.0F}));
    assert(!IsValidRequest(WeatherRequest{1.0F, 0.0F, 11.0F, 1.0F}));
    assert(!IsValidRequest(WeatherRequest{1.0F, 0.0F, 0.0F, 1.5F}));
    assert(!IsValidRequest(WeatherRequest{
        std::numeric_limits<float>::quiet_NaN(), 0.0F, 0.0F, 1.0F}));
    return 0;
}
