import assert from "node:assert/strict";

import { formatTelemetrySummary, setTelemetrySessionState } from "./telemetry-display.js";

function completeSample() {
  return {

    pose: { xM: 1.25, yM: -2.5, headingRad: 0.75 },
    velocity: { linearXMps: 3.5, angularRadS: -0.25 },
  };
}

function testCompleteSampleFormatsMeasuredValues() {
  assert.equal(
    formatTelemetrySummary(completeSample(), { armState: 2, ageMs: 100, stale: false }),
    "ARMED | pose x=1.25m y=-2.50m heading=0.75rad | v=3.50m/s w=-0.25rad/s",
  );
}

function testMissingGroupsNeverBecomeZeroMeasurements() {
  const text = formatTelemetrySummary({ pose: null, velocity: null }, { armState: 1, ageMs: 100, stale: false });
  assert.equal(text, "DISARMED | pose Missing | velocity Missing");
  assert.equal(text.includes("0.00"), false);
}

function testNonFiniteLegacyValuesFailVisibly() {
  const sample = completeSample();
  sample.pose.headingRad = Number.NaN;
  sample.velocity.linearXMps = Number.POSITIVE_INFINITY;
  assert.equal(
    formatTelemetrySummary(sample, { armState: 2, ageMs: 100, stale: false }),
    "ARMED | pose Invalid | velocity Invalid",
  );
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

function truthSample(overrides = {}) {
  return {
    pose: null,
    velocity: null,
    simTruth: {
      posNed: [1, 2, 3],
      velNed: [0, 0, 0],
      quat: { w: 1, x: 0, y: 0, z: 0 },
      validFlags: 0b1101,
      stamp: {
        sourceId: 1n,
        sourceIncarnation: "11".repeat(16),
        sourceEpoch: 2,
        sequence: 40,
        acquiredAtNanos: 1_000_000n,
        clock: 2,
        role: 2,
        integrity: 3,
      },
      ...overrides,
    },
  };
}

function testArmFreshnessStates() {
  assert.match(formatTelemetrySummary({ pose: null, velocity: null }), /^arm: missing/);
  assert.match(
    formatTelemetrySummary({ pose: null, velocity: null }, { armState: 2, ageMs: 9000, stale: true }),
    /^arm: stale/,
  );
}
testArmFreshnessStates();
console.log("ok - testArmFreshnessStates");

function testTruthGatesOnValidityIntegrityAndRole() {
  assert.match(
    formatTelemetrySummary(truthSample()),
    /SIM truth \(oracle\): n=1\.00m e=2\.00m d=3\.00m$/,
  );
  // Position availability cleared -> Unavailable, never rendered values.
  assert.match(
    formatTelemetrySummary(truthSample({ validFlags: 0 })),
    /SIM truth \(oracle\): Unavailable$/,
  );
  // Unspecified integrity -> Unavailable.
  assert.match(
    formatTelemetrySummary(
      truthSample({ stamp: { ...truthSample().simTruth.stamp, integrity: 0 } }),
    ),
    /SIM truth \(oracle\): Unavailable$/,
  );
  // Wrong role -> Unavailable even if a decoder let it through.
  assert.match(
    formatTelemetrySummary(
      truthSample({ stamp: { ...truthSample().simTruth.stamp, role: 3 } }),
    ),
    /SIM truth \(oracle\): Unavailable$/,
  );
  // A non-simulation clock on a truth stamp -> Unavailable.
  assert.match(
    formatTelemetrySummary(
      truthSample({ stamp: { ...truthSample().simTruth.stamp, clock: 1 } }),
    ),
    /SIM truth \(oracle\): Unavailable$/,
  );
}
testTruthGatesOnValidityIntegrityAndRole();
console.log("ok - testTruthGatesOnValidityIntegrityAndRole");
