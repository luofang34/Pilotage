import assert from "node:assert/strict";

import {
  AvionicsIngress,
  COHERENCE,
  INCARNATION_POLICY,
  serialIsNewer,
} from "./telemetry-ingress.js";

const VEHICLE = 1n;
const SOURCE = 7n;
const CLOCK = 1;
const INCARNATION_A = "a5".repeat(16);
const INCARNATION_B = "5a".repeat(16);
const INCARNATION_C = "3c".repeat(16);

function stamp(
  sequence,
  acquiredAtNanos,
  sourceEpoch = 1,
  sourceId = SOURCE,
  sourceIncarnation = INCARNATION_A,
) {
  return { sourceId, sourceIncarnation, sourceEpoch, sequence, acquiredAtNanos, clock: CLOCK };
}

function avionics(attitudeStamp, kinematicsStamp, value = 1) {
  return {
    quat: { w: value, x: 0, y: 0, z: 0 },
    rates: [value, 0, 0],
    posNed: [value, 0, 0],
    velNed: [value, 0, 0],
    validFlags: 0b1111,
    quality: 0,
    armState: 2,
    attitudeStamp,
    kinematicsStamp,
  };
}

function packet(attitudeStamp, kinematicsStamp, value = 1, vehicleId = VEHICLE) {
  return { vehicleId, avionics: avionics(attitudeStamp, kinematicsStamp, value) };
}

function ingress(maximumSkewNanos = 100n) {
  return new AvionicsIngress({ vehicleId: VEHICLE, maximumSkewNanos });
}

function testDuplicateDoesNotRefreshAge() {
  const gate = ingress();
  const first = packet(stamp(10, 1_000n), stamp(20, 1_000n));
  assert.equal(gate.ingest(first, 100), true);
  assert.equal(gate.ingest(first, 600), false);
  assert.equal(gate.snapshot(850).attitude.ageMs, 750);
  assert.equal(gate.snapshot(3_100).attitude.ageMs, 3_000);
  const snapshot = gate.snapshot(1_100);
  assert.equal(snapshot.attitude.ageMs, 1_000);
  assert.equal(snapshot.kinematics.ageMs, 1_000);
  assert.equal(gate.diagnostics().duplicates, 2);
}

function testReorderAndGapHandling() {
  const gate = ingress();
  assert.equal(gate.ingest(packet(stamp(10, 100n), null), 0), true);
  assert.equal(gate.ingest(packet(stamp(12, 120n), null, 2), 20), true);
  assert.equal(gate.ingest(packet(stamp(11, 110n), null, 3), 30), false);
  const snapshot = gate.snapshot(30);
  assert.equal(snapshot.attitude.quat.w, 2);
  assert.equal(gate.diagnostics().sequenceGaps, 1);
  assert.equal(gate.diagnostics().reordered, 1);
}

function testSequenceAndEpochWrap() {
  assert.equal(serialIsNewer(0, 0xffff_ffff), true);
  assert.equal(serialIsNewer(0xffff_ffff, 0), false);
  assert.equal(serialIsNewer(0x8000_0000, 0), false);
  const gate = ingress();
  gate.ingest(packet(stamp(0xffff_ffff, 10n), null), 0);
  assert.equal(gate.ingest(packet(stamp(0, 20n), null, 2), 1), true);
  assert.equal(gate.snapshot(1).attitude.quat.w, 2);

  assert.equal(gate.ingest(packet(stamp(0, 1n, 2), stamp(0, 1n, 2), 3), 2), true);
  const reset = gate.snapshot(2);
  assert.equal(reset.sourceEpoch, 2);
  assert.equal(reset.attitude.quat.w, 3);
  assert.equal(reset.kinematics.posNed[0], 3);
  assert.equal(gate.diagnostics().sourceResets, 1);
  assert.equal(gate.ingest(packet(stamp(1, 30n, 1), null, 4), 3), false);
  assert.equal(gate.diagnostics().oldEpoch, 1);

  const epochGate = ingress();
  epochGate.ingest(packet(stamp(0, 10n, 0xffff_ffff), null), 0);
  assert.equal(epochGate.ingest(packet(stamp(0, 20n, 0), null, 5), 1), true);
  assert.equal(epochGate.snapshot(1).sourceEpoch, 0);
  assert.equal(epochGate.diagnostics().sourceResets, 1);
}

function testVehicleAndSourceIsolation() {
  const gate = ingress();
  assert.equal(gate.ingest(packet(stamp(1, 10n), null, 1, 2n), 0), false);
  assert.equal(gate.ingest(packet(stamp(1, 10n), null), 1), true);
  assert.equal(gate.ingest(packet(stamp(2, 20n, 1, 8n), null, 9), 2), false);
  assert.equal(gate.snapshot(2).attitude.quat.w, 1);
  assert.equal(gate.diagnostics().wrongVehicle, 1);
  assert.equal(gate.diagnostics().wrongSource, 1);
}

function testDefaultPolicyPinsFirstIncarnation() {
  const gate = ingress();
  assert.equal(gate.ingest(packet(stamp(1, 10n), null), 0), true);
  assert.equal(
    gate.ingest(packet(stamp(0, 1n, 1, SOURCE, INCARNATION_B), null, 2), 1),
    false,
  );
  assert.equal(gate.snapshot(1).attitude.quat.w, 1);
  assert.equal(gate.diagnostics().wrongIncarnation, 1);
}

function testSimulatorPolicyAcceptsUnseenAndRejectsSeenIncarnations() {
  const gate = new AvionicsIngress({
    vehicleId: VEHICLE,
    maximumSkewNanos: 100n,
    incarnationPolicy: INCARNATION_POLICY.SIM_ACCEPT_UNSEEN,
    maximumSeenIncarnations: 2,
  });
  gate.ingest(packet(stamp(1, 10n), null), 0);
  assert.equal(
    gate.ingest(packet(stamp(0, 1n, 1, SOURCE, INCARNATION_B), null, 2), 1),
    true,
  );
  assert.equal(gate.snapshot(1).sourceIncarnation, INCARNATION_B);
  assert.equal(gate.diagnostics().incarnationTransitions, 1);
  assert.equal(gate.ingest(packet(stamp(2, 20n), null, 3), 2), false);
  assert.equal(gate.diagnostics().oldIncarnation, 1);
  assert.equal(
    gate.ingest(packet(stamp(0, 1n, 1, SOURCE, INCARNATION_C), null, 4), 3),
    false,
  );
  assert.equal(gate.diagnostics().incarnationCapacity, 1);
}

function testAcquisitionTimeRegressionDoesNotRefreshAge() {
  const gate = ingress();
  gate.ingest(packet(stamp(10, 1_000n), null), 100);
  assert.equal(gate.ingest(packet(stamp(11, 900n), null, 2), 500), false);
  const snapshot = gate.snapshot(850);
  assert.equal(snapshot.attitude.quat.w, 1);
  assert.equal(snapshot.attitude.ageMs, 750);
  assert.equal(gate.diagnostics().timeRegressions, 1);

  assert.equal(gate.ingest(packet(stamp(12, 1_000n), null, 3), 900), false);
  assert.equal(gate.snapshot(1_100).attitude.ageMs, 1_000);
  assert.equal(gate.diagnostics().timeRegressions, 2);
}

function testIndependentGroupsAndCoherenceBoundary() {
  const gate = ingress(100n);
  gate.ingest(packet(stamp(1, 1_000n), stamp(1, 1_100n)), 10);
  let snapshot = gate.snapshot(10);
  assert.equal(snapshot.coherence.status, COHERENCE.COHERENT);
  assert.equal(snapshot.coherence.skewNanos, 100n);

  gate.ingest(packet(stamp(2, 1_500n), null, 2), 20);
  snapshot = gate.snapshot(30);
  assert.equal(snapshot.attitude.ageMs, 10);
  assert.equal(snapshot.kinematics.ageMs, 20);
  assert.equal(snapshot.coherence.status, COHERENCE.EXCESSIVE_SKEW);
  assert.equal(gate.diagnostics().excessiveSkew, 1);

  gate.ingest(packet(null, stamp(2, 1_450n), 3), 40);
  snapshot = gate.snapshot(40);
  assert.equal(snapshot.coherence.status, COHERENCE.COHERENT);
  assert.equal(snapshot.attitude.quat.w, 2);
  assert.equal(snapshot.kinematics.posNed[0], 3);
}

function testSnapshotsAreAtomicAndImmutable() {
  const gate = ingress();
  gate.ingest(packet(stamp(1, 10n), stamp(1, 10n), 1), 0);
  const before = gate.snapshot(0);
  gate.ingest(packet(stamp(2, 20n), stamp(2, 20n), 2), 1);
  const after = gate.snapshot(1);

  assert.equal(before.generation, 1);
  assert.equal(before.attitude.quat.w, 1);
  assert.equal(before.kinematics.posNed[0], 1);
  assert.equal(after.generation, 2);
  assert.equal(after.attitude.quat.w, 2);
  assert.equal(after.kinematics.posNed[0], 2);
  assert.equal(Object.isFrozen(before), true);
  assert.equal(Object.isFrozen(before.attitude.quat), true);
}

function testInvalidStampIsRejected() {
  const gate = ingress();
  const invalid = { ...stamp(1, 10n), clock: 0 };
  assert.equal(gate.ingest(packet(invalid, null), 0), false);
  assert.equal(gate.snapshot(0).attitude, null);
  assert.equal(gate.diagnostics().invalidStamps, 1);
  assert.equal(gate.ingest(packet({ ...stamp(1, 10n), sourceEpoch: -1 }, null), 1), false);
  assert.equal(
    gate.ingest(packet({ ...stamp(1, 10n), sequence: 0x1_0000_0000 }, null), 2),
    false,
  );
  assert.equal(gate.diagnostics().invalidStamps, 3);
}

function testCountersAndGenerationWrap() {
  const gate = ingress();
  gate.generation = 0xffff_ffff;
  gate.counters.duplicates = 0xffff_ffff;
  const first = packet(stamp(1, 10n), null);
  assert.equal(gate.ingest(first, 0), true);
  assert.equal(gate.snapshot(0).generation, 0);
  assert.equal(gate.ingest(first, 1), false);
  assert.equal(gate.diagnostics().duplicates, 0);
}

for (const test of [
  testDuplicateDoesNotRefreshAge,
  testReorderAndGapHandling,
  testSequenceAndEpochWrap,
  testVehicleAndSourceIsolation,
  testDefaultPolicyPinsFirstIncarnation,
  testSimulatorPolicyAcceptsUnseenAndRejectsSeenIncarnations,
  testAcquisitionTimeRegressionDoesNotRefreshAge,
  testIndependentGroupsAndCoherenceBoundary,
  testSnapshotsAreAtomicAndImmutable,
  testInvalidStampIsRejected,
  testCountersAndGenerationWrap,
]) {
  test();
  console.log(`ok - ${test.name}`);
}
