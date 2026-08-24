#include "weather_state.h"

#include <cassert>
#include <cmath>
#include <limits>

using pilotage::weather::CalmPayload;
using pilotage::weather::kMaximumGeneration;
using pilotage::weather::PendingWeatherRequest;
using pilotage::weather::RequestStatus;
using pilotage::weather::WeatherOperation;
using pilotage::weather::WeatherPayload;
using pilotage::weather::WeatherTransactionState;

namespace {

WeatherPayload Wind(float speed) {
    return WeatherPayload{speed, 270.0F, 0.2F};
}

void AssertPayload(const WeatherPayload& actual,
                   const WeatherPayload& expected) {
    assert(actual.wind_speed_mps == expected.wind_speed_mps);
    assert(actual.wind_direction_deg == expected.wind_direction_deg);
    assert(actual.turbulence_scale == expected.turbulence_scale);
}

PendingWeatherRequest TakePending(WeatherTransactionState* state) {
    PendingWeatherRequest request;
    assert(state->TakePending(&request));
    return request;
}

void TestIdleAndGenerationValidation() {
    WeatherTransactionState state;
    PendingWeatherRequest request;
    assert(state.status() == RequestStatus::Idle);
    assert(state.expected_generation() == 1);
    assert(state.response_generation() == 0);
    assert(state.applied_generation() == 0);
    assert(!state.TakePending(&request));

    state.TriggerApply(0.0F, Wind(5.0F));
    assert(state.status() == RequestStatus::Invalid);
    assert(state.expected_generation() == 1);
    state.TriggerApply(1.5F, Wind(5.0F));
    assert(state.status() == RequestStatus::Invalid);
    state.TriggerApply(std::numeric_limits<float>::quiet_NaN(), Wind(5.0F));
    assert(state.status() == RequestStatus::Invalid);
    state.TriggerApply(16777216.0F, Wind(5.0F));
    assert(state.status() == RequestStatus::Invalid);
    state.TriggerApply(16777215.0F, Wind(5.0F));
    assert(state.status() == RequestStatus::OutOfSequenceGeneration);
    assert(state.expected_generation() == 1);
}

void TestApplyBindsAnImmutableRequest() {
    WeatherTransactionState state;
    WeatherPayload staged = Wind(5.0F);
    state.TriggerApply(1.0F, staged);
    staged.wind_speed_mps = 9.0F;

    assert(state.status() == RequestStatus::Pending);
    assert(state.expected_generation() == 2);
    assert(state.response_generation() == 1);
    assert(state.response_operation() == WeatherOperation::Apply);
    AssertPayload(state.response_payload(), Wind(5.0F));

    const PendingWeatherRequest request = TakePending(&state);
    assert(request.operation == WeatherOperation::Apply);
    assert(request.generation == 1);
    AssertPayload(request.payload, Wind(5.0F));
    state.Complete(request);

    assert(state.status() == RequestStatus::Applied);
    assert(state.applied_generation() == 1);
    AssertPayload(state.response_payload(), Wind(5.0F));
}

void TestGenerationFailuresDoNotChangeAppliedIdentity() {
    WeatherTransactionState state;
    state.TriggerApply(1.0F, Wind(5.0F));
    state.Complete(TakePending(&state));

    state.TriggerApply(1.0F, Wind(7.0F));
    assert(state.status() == RequestStatus::StaleGeneration);
    assert(state.response_generation() == 1);
    AssertPayload(state.response_payload(), Wind(7.0F));
    assert(state.applied_generation() == 1);

    state.TriggerApply(3.0F, Wind(7.0F));
    assert(state.status() == RequestStatus::OutOfSequenceGeneration);
    assert(state.response_generation() == 3);
    assert(state.expected_generation() == 2);
    assert(state.applied_generation() == 1);

    state.TriggerApply(2.0F, Wind(101.0F));
    assert(state.status() == RequestStatus::Invalid);
    assert(state.response_generation() == 2);
    assert(state.expected_generation() == 3);
    assert(state.applied_generation() == 1);

    state.TriggerApply(3.0F, Wind(7.0F));
    const PendingWeatherRequest request = TakePending(&state);
    state.Fail(request, RequestStatus::DatarefMissing);
    assert(state.status() == RequestStatus::DatarefMissing);
    assert(state.response_generation() == 3);
    assert(state.expected_generation() == 4);
    assert(state.applied_generation() == 1);
}

void TestClearUsesCanonicalCalmAndHasItsOwnAck() {
    WeatherTransactionState state;
    state.TriggerApply(1.0F, Wind(5.0F));
    state.Complete(TakePending(&state));

    state.TriggerClear(2.0F);
    const PendingWeatherRequest request = TakePending(&state);
    assert(request.operation == WeatherOperation::Clear);
    assert(request.generation == 2);
    AssertPayload(request.payload, CalmPayload());
    state.Complete(request);

    assert(state.status() == RequestStatus::Cleared);
    assert(state.applied_generation() == 2);
    assert(state.response_operation() == WeatherOperation::Clear);
    AssertPayload(state.response_payload(), CalmPayload());
}

void TestBusyRequestCanRetryItsExpectedGeneration() {
    WeatherTransactionState state;
    state.TriggerApply(1.0F, Wind(5.0F));
    state.TriggerClear(2.0F);
    assert(state.status() == RequestStatus::Busy);
    assert(state.response_generation() == 2);
    assert(state.expected_generation() == 2);

    state.Complete(TakePending(&state));
    state.TriggerClear(2.0F);
    state.Complete(TakePending(&state));
    assert(state.status() == RequestStatus::Cleared);
    assert(state.applied_generation() == 2);
}

void TestResetReturnsToIdleWithoutAnImplicitRequest() {
    WeatherTransactionState state;
    state.TriggerApply(1.0F, Wind(5.0F));
    state.Complete(TakePending(&state));
    state.Reset();

    PendingWeatherRequest request;
    assert(state.status() == RequestStatus::Idle);
    assert(state.expected_generation() == 1);
    assert(state.response_generation() == 0);
    assert(state.applied_generation() == 0);
    assert(!state.TakePending(&request));
}

void TestDisableRefusesRequestsAndEnableReturnsToIdle() {
    WeatherTransactionState state;
    state.TriggerApply(1.0F, Wind(5.0F));
    state.Complete(TakePending(&state));
    state.Disable();

    state.TriggerApply(1.0F, Wind(7.0F));
    assert(state.status() == RequestStatus::Unavailable);
    assert(state.applied_generation() == 0);
    assert(state.expected_generation() == 1);

    state.Enable();
    PendingWeatherRequest request;
    assert(state.status() == RequestStatus::Idle);
    assert(!state.TakePending(&request));
    state.TriggerApply(1.0F, Wind(7.0F));
    assert(state.status() == RequestStatus::Pending);
}

void TestGenerationExhaustionFailsClosed() {
    WeatherTransactionState state;
    std::uint32_t generation = 1;
    while (true) {
        state.TriggerClear(static_cast<float>(generation));
        TakePending(&state);
        if (generation == kMaximumGeneration) {
            break;
        }
        generation += 1;
    }
    assert(state.expected_generation() == 0);
    state.TriggerClear(1.0F);
    assert(state.status() == RequestStatus::GenerationExhausted);
    assert(state.applied_generation() == 0);
}

}  // namespace

int main() {
    TestIdleAndGenerationValidation();
    TestApplyBindsAnImmutableRequest();
    TestGenerationFailuresDoNotChangeAppliedIdentity();
    TestClearUsesCanonicalCalmAndHasItsOwnAck();
    TestBusyRequestCanRetryItsExpectedGeneration();
    TestResetReturnsToIdleWithoutAnImplicitRequest();
    TestDisableRefusesRequestsAndEnableReturnsToIdle();
    TestGenerationExhaustionFailsClosed();
    return 0;
}
