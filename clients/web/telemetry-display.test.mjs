import assert from "node:assert/strict";

import { formatTelemetrySummary, setTelemetrySessionState } from "./telemetry-display.js";

function completeSample() {
  return {
    avionics: { armState: 2 },
    pose: { xM: 1.25, yM: -2.5, headingRad: 0.75 },
    velocity: { linearXMps: 3.5, angularRadS: -0.25 },
  };
}

function testCompleteSampleFormatsMeasuredValues() {
  assert.equal(
    formatTelemetrySummary(completeSample()),
    "ARMED | pose x=1.25m y=-2.50m heading=0.75rad | v=3.50m/s w=-0.25rad/s",
  );
}

function testMissingGroupsNeverBecomeZeroMeasurements() {
  const text = formatTelemetrySummary({ avionics: { armState: 1 }, pose: null, velocity: null });
  assert.equal(text, "DISARMED | pose Missing | velocity Missing");
  assert.equal(text.includes("0.00"), false);
}

function testNonFiniteLegacyValuesFailVisibly() {
  const sample = completeSample();
  sample.pose.headingRad = Number.NaN;
  sample.velocity.linearXMps = Number.POSITIVE_INFINITY;
  assert.equal(formatTelemetrySummary(sample), "ARMED | pose Invalid | velocity Invalid");
}

function testSessionTransitionsRetireDisplayedMeasurements() {
  const elements = {
    telemetry: { textContent: "pose x=42.00m" },
    overlay: { textContent: "authority: old session" },
  };
  setTelemetrySessionState(elements, "connecting");
  assert.deepEqual(elements, {
    telemetry: { textContent: "telemetry Missing (connecting)" },
    overlay: { textContent: "connecting" },
  });
  setTelemetrySessionState(elements, "disconnected");
  assert.deepEqual(elements, {
    telemetry: { textContent: "telemetry Missing (disconnected)" },
    overlay: { textContent: "disconnected" },
  });
}

for (const test of [
  testCompleteSampleFormatsMeasuredValues,
  testMissingGroupsNeverBecomeZeroMeasurements,
  testNonFiniteLegacyValuesFailVisibly,
  testSessionTransitionsRetireDisplayedMeasurements,
]) {
  test();
  console.log(`ok - ${test.name}`);
}
