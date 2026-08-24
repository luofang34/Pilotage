#pragma once

#include <cstdint>

namespace pilotage::weather {

constexpr float kMaximumWindSpeedMps = 100.0F;
constexpr float kMaximumTurbulenceScale = 10.0F;
constexpr std::uint32_t kMaximumGeneration = 16777215U;

struct WeatherPayload {
    float wind_speed_mps = 0.0F;
    float wind_direction_deg = 0.0F;
    float turbulence_scale = 0.0F;
};

enum class WeatherOperation {
    None = 0,
    Apply = 1,
    Clear = 2,
};

enum class RequestStatus {
    Idle = 0,
    Applied = 1,
    Pending = 2,
    Cleared = 3,
    Invalid = -1,
    DatarefMissing = -2,
    StaleGeneration = -3,
    OutOfSequenceGeneration = -4,
    ReadbackMismatch = -5,
    Busy = -6,
    GenerationExhausted = -7,
    Unavailable = -8,
};

struct PendingWeatherRequest {
    WeatherOperation operation = WeatherOperation::None;
    std::uint32_t generation = 0;
    WeatherPayload payload{};
};

class WeatherTransactionState {
  public:
    void TriggerApply(float generation, const WeatherPayload& staged);
    void TriggerClear(float generation);
    bool TakePending(PendingWeatherRequest* request);
    void Complete(const PendingWeatherRequest& request);
    void Fail(const PendingWeatherRequest& request, RequestStatus failure);
    void Enable();
    void Disable();
    void Reset();

    [[nodiscard]] std::uint32_t expected_generation() const;
    [[nodiscard]] std::uint32_t response_generation() const;
    [[nodiscard]] std::uint32_t applied_generation() const;
    [[nodiscard]] WeatherOperation response_operation() const;
    [[nodiscard]] const WeatherPayload& response_payload() const;
    [[nodiscard]] RequestStatus status() const;

  private:
    void Trigger(WeatherOperation operation, float generation,
                 const WeatherPayload& payload);
    void SetResponse(WeatherOperation operation, std::uint32_t generation,
                     const WeatherPayload& payload, RequestStatus status);
    void AdvanceExpectedGeneration();

    std::uint32_t expected_generation_ = 1;
    std::uint32_t response_generation_ = 0;
    std::uint32_t applied_generation_ = 0;
    WeatherOperation response_operation_ = WeatherOperation::None;
    WeatherPayload response_payload_{};
    RequestStatus status_ = RequestStatus::Idle;
    PendingWeatherRequest pending_{};
    bool enabled_ = true;
};

[[nodiscard]] bool IsValidPayload(const WeatherPayload& payload);
[[nodiscard]] WeatherPayload CalmPayload();

}  // namespace pilotage::weather
