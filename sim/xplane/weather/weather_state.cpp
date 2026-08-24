#include "weather_state.h"

#include <cmath>

namespace pilotage::weather {
namespace {

bool DecodeGeneration(float value, std::uint32_t* generation) {
    if (!std::isfinite(value) || value < 1.0F ||
        value > static_cast<float>(kMaximumGeneration) ||
        std::floor(value) != value) {
        return false;
    }
    *generation = static_cast<std::uint32_t>(value);
    return true;
}

}  // namespace

bool IsValidPayload(const WeatherPayload& payload) {
    return std::isfinite(payload.wind_speed_mps) &&
           payload.wind_speed_mps >= 0.0F &&
           payload.wind_speed_mps <= kMaximumWindSpeedMps &&
           std::isfinite(payload.wind_direction_deg) &&
           payload.wind_direction_deg >= 0.0F &&
           payload.wind_direction_deg <= 360.0F &&
           std::isfinite(payload.turbulence_scale) &&
           payload.turbulence_scale >= 0.0F &&
           payload.turbulence_scale <= kMaximumTurbulenceScale;
}

WeatherPayload CalmPayload() { return WeatherPayload{}; }

void WeatherTransactionState::TriggerApply(
    float generation, const WeatherPayload& staged) {
    Trigger(WeatherOperation::Apply, generation, staged);
}

void WeatherTransactionState::TriggerClear(float generation) {
    Trigger(WeatherOperation::Clear, generation, CalmPayload());
}

void WeatherTransactionState::Trigger(WeatherOperation operation,
                                      float generation,
                                      const WeatherPayload& payload) {
    std::uint32_t decoded = 0;
    if (!DecodeGeneration(generation, &decoded)) {
        SetResponse(operation, 0, payload, RequestStatus::Invalid);
        return;
    }
    if (!enabled_) {
        SetResponse(operation, decoded, payload, RequestStatus::Unavailable);
        return;
    }
    if (pending_.operation != WeatherOperation::None) {
        SetResponse(operation, decoded, payload, RequestStatus::Busy);
        return;
    }
    if (expected_generation_ == 0) {
        SetResponse(operation, decoded, payload,
                    RequestStatus::GenerationExhausted);
        return;
    }
    if (decoded < expected_generation_) {
        SetResponse(operation, decoded, payload,
                    RequestStatus::StaleGeneration);
        return;
    }
    if (decoded > expected_generation_) {
        SetResponse(operation, decoded, payload,
                    RequestStatus::OutOfSequenceGeneration);
        return;
    }

    SetResponse(operation, decoded, payload, RequestStatus::Pending);
    AdvanceExpectedGeneration();
    if (operation == WeatherOperation::Apply && !IsValidPayload(payload)) {
        status_ = RequestStatus::Invalid;
        return;
    }
    pending_ = PendingWeatherRequest{operation, decoded, payload};
}

bool WeatherTransactionState::TakePending(PendingWeatherRequest* request) {
    if (request == nullptr || pending_.operation == WeatherOperation::None) {
        return false;
    }
    *request = pending_;
    pending_ = PendingWeatherRequest{};
    return true;
}

void WeatherTransactionState::Complete(
    const PendingWeatherRequest& request) {
    applied_generation_ = request.generation;
    SetResponse(request.operation, request.generation, request.payload,
                request.operation == WeatherOperation::Clear
                    ? RequestStatus::Cleared
                    : RequestStatus::Applied);
}

void WeatherTransactionState::Fail(const PendingWeatherRequest& request,
                                   RequestStatus failure) {
    SetResponse(request.operation, request.generation, request.payload, failure);
}

void WeatherTransactionState::Enable() {
    if (!enabled_) {
        *this = WeatherTransactionState{};
    }
}

void WeatherTransactionState::Disable() {
    *this = WeatherTransactionState{};
    enabled_ = false;
    status_ = RequestStatus::Unavailable;
}

void WeatherTransactionState::Reset() { *this = WeatherTransactionState{}; }

std::uint32_t WeatherTransactionState::expected_generation() const {
    return expected_generation_;
}

std::uint32_t WeatherTransactionState::response_generation() const {
    return response_generation_;
}

std::uint32_t WeatherTransactionState::applied_generation() const {
    return applied_generation_;
}

WeatherOperation WeatherTransactionState::response_operation() const {
    return response_operation_;
}

const WeatherPayload& WeatherTransactionState::response_payload() const {
    return response_payload_;
}

RequestStatus WeatherTransactionState::status() const { return status_; }

void WeatherTransactionState::SetResponse(WeatherOperation operation,
                                          std::uint32_t generation,
                                          const WeatherPayload& payload,
                                          RequestStatus status) {
    response_operation_ = operation;
    response_generation_ = generation;
    response_payload_ = payload;
    status_ = status;
}

void WeatherTransactionState::AdvanceExpectedGeneration() {
    if (expected_generation_ == kMaximumGeneration) {
        expected_generation_ = 0;
    } else {
        ++expected_generation_;
    }
}

}  // namespace pilotage::weather
