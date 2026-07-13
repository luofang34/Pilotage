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
    estimatorStatusStamp: null,
  };
}

function packet(attitudeStamp, kinematicsStamp, value = 1, vehicleId = VEHICLE) {
  return { vehicleId, avionics: avionics(attitudeStamp, kinematicsStamp, value) };
}

function statusPacket(statusStamp, validFlags, quality, vehicleId = VEHICLE) {
  return {
    vehicleId,
    avionics: {
      ...avionics(null, null),
      validFlags,
      quality,
      estimatorStatusStamp: statusStamp,
    },
  };
}

function pairedPacket(
  attitudeStamp,
  kinematicsStamp,
  statusStamp,
  value,
  validFlags,
  quality,
) {
  return {
    vehicleId: VEHICLE,
    avionics: {
      ...avionics(attitudeStamp, kinematicsStamp, value),
      validFlags,
      quality,
      estimatorStatusStamp: statusStamp,
    },
  };
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

function testStatusOnlyRevocationDoesNotRefreshMeasurements() {
  const gate = ingress();
  const initialStatus = stamp(10, 1_000n);
  gate.ingest(statusPacket(initialStatus, 0b1111, 0), 10);
  let snapshot = gate.snapshot(10);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);

  gate.ingest(
    pairedPacket(stamp(1, 1_000n), stamp(1, 1_000n), initialStatus, 1, 0b1111, 0),
    15,
  );
  snapshot = gate.snapshot(20);
  assert.equal(snapshot.generation, 2);
  assert.equal(snapshot.validFlags, 0b1111);
  assert.equal(snapshot.quality, 0);
  assert.equal(snapshot.attitude.ageMs, 5);
  assert.equal(snapshot.estimatorStatus.ageMs, 10);

  assert.equal(gate.ingest(statusPacket(stamp(11, 1_100n), 0, 2), 30), true);
  snapshot = gate.snapshot(40);
  assert.equal(snapshot.generation, 3);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);
  assert.equal(snapshot.attitude.ageMs, 25);
  assert.equal(snapshot.kinematics.ageMs, 25);
  assert.equal(snapshot.estimatorStatus.ageMs, 10);
}

function testStatusPayloadAloneControlsAuthorization() {
  const gate = ingress();
  const unusableStatus = stamp(10, 1_000n);
  gate.ingest(statusPacket(unusableStatus, 0, 2), 0);
  gate.ingest(pairedPacket(stamp(1, 1_000n), null, unusableStatus, 1, 0, 2), 1);

  const goodStatus = stamp(11, 1_100n);
  gate.ingest(statusPacket(goodStatus, 0b1111, 0), 2);
  let snapshot = gate.snapshot(2);
  assert.equal(snapshot.quality, 2);
  assert.equal(snapshot.validFlags, 0);
  assert.equal("quality" in snapshot.estimatorStatus, false);
  assert.equal("validFlags" in snapshot.estimatorStatus, false);

  gate.ingest(pairedPacket(stamp(2, 1_050n), null, goodStatus, 2, 0b0011, 0), 3);
  snapshot = gate.snapshot(3);
  assert.equal(snapshot.quality, 2);
  assert.equal(snapshot.validFlags, 0);

  gate.ingest(pairedPacket(stamp(3, 1_100n), null, goodStatus, 3, 0b0011, 0), 4);
  snapshot = gate.snapshot(4);
  assert.equal(snapshot.quality, 0);
  assert.equal(snapshot.validFlags, 0b0011);

  gate.ingest(statusPacket(stamp(12, 1_300n), 0b0001, 1), 5);
  gate.ingest(statusPacket(stamp(13, 1_400n), 0b1111, 0), 6);
  snapshot = gate.snapshot(6);
  assert.equal(snapshot.quality, 1);
  assert.equal(snapshot.validFlags, 0b0001);
}

function testMismatchedGroupCannotRegainThroughOtherNumeric() {
  const gate = ingress();
  const firstStatus = stamp(10, 1_000n);
  gate.ingest(statusPacket(firstStatus, 0b1111, 0), 0);
  gate.ingest(
    pairedPacket(stamp(1, 1_000n), stamp(1, 1_000n), firstStatus, 1, 0b1111, 0),
    1,
  );

  const nextStatus = stamp(11, 1_200n);
  gate.ingest(statusPacket(nextStatus, 0b1111, 0), 2);
  gate.ingest(pairedPacket(stamp(2, 1_100n), null, nextStatus, 2, 0b1111, 0), 3);
  assert.equal(gate.snapshot(3).validFlags, 0b1100);

  gate.ingest(pairedPacket(null, stamp(2, 1_200n), nextStatus, 3, 0b1111, 0), 4);
  assert.equal(gate.snapshot(4).validFlags, 0b1100);

  gate.ingest(pairedPacket(stamp(3, 1_200n), null, nextStatus, 4, 0b1111, 0), 5);
  assert.equal(gate.snapshot(5).validFlags, 0b1111);
}

function testStatusGoodReauthorizesOnlyExactNumericGroup() {
  const gate = ingress();
  const firstStatus = stamp(10, 1_000n);
  gate.ingest(statusPacket(firstStatus, 0b1111, 0), 0);
  gate.ingest(
    pairedPacket(stamp(1, 1_000n), stamp(1, 1_000n), firstStatus, 1, 0b1111, 0),
    1,
  );

  gate.ingest(statusPacket(stamp(11, 1_100n), 0, 2), 2);
  const goodStatus = stamp(12, 1_200n);
  gate.ingest(statusPacket(goodStatus, 0b1111, 0), 3);
  let snapshot = gate.snapshot(3);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);

  gate.ingest(pairedPacket(null, stamp(2, 1_200n), goodStatus, 2, 0b1111, 0), 4);
  snapshot = gate.snapshot(4);
  assert.equal(snapshot.validFlags, 0b1100);
  assert.equal(snapshot.quality, 0);

  gate.ingest(pairedPacket(stamp(2, 1_200n), null, goodStatus, 3, 0b1111, 0), 5);
  assert.equal(gate.snapshot(5).validFlags, 0b1111);
}

function testCombinedStatusRevokesBeforeOneNumericGroupRecovers() {
  const gate = ingress();
  const firstStatus = stamp(10, 1_000n);
  gate.ingest(statusPacket(firstStatus, 0b1111, 0), 0);
  gate.ingest(
    pairedPacket(stamp(1, 1_000n), stamp(1, 1_000n), firstStatus, 1, 0b1111, 0),
    1,
  );

  const nextStatus = stamp(11, 1_200n);
  gate.ingest(
    pairedPacket(stamp(2, 1_200n), stamp(1, 1_000n), nextStatus, 2, 0b0011, 0),
    2,
  );
  let snapshot = gate.snapshot(2);
  assert.equal(snapshot.validFlags, 0b0011);
  assert.equal(snapshot.quality, 0);

  const unusableStatus = stamp(12, 1_300n);
  gate.ingest(
    pairedPacket(stamp(3, 1_250n), stamp(1, 1_000n), unusableStatus, 3, 0, 2),
    3,
  );
  snapshot = gate.snapshot(3);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);
}

function testDuplicateStatusStampCanOnlyPublishALocalDowngrade() {
  const gate = ingress();
  const status = stamp(10, 1_000n);
  const good = pairedPacket(stamp(1, 1_000n), stamp(1, 1_000n), status, 1, 0b1111, 0);
  gate.ingest(statusPacket(status, 0b1111, 0), 0);
  gate.ingest(good, 1);
  const generation = gate.snapshot(1).generation;

  const revoked = pairedPacket(stamp(1, 1_000n), stamp(1, 1_000n), status, 1, 0, 2);
  assert.equal(gate.ingest(revoked, 10), true);
  let snapshot = gate.snapshot(11);
  assert.equal(snapshot.generation, (generation + 1) >>> 0);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);
  assert.equal(snapshot.attitude.ageMs, 10);
  assert.equal(snapshot.estimatorStatus.ageMs, 11);

  assert.equal(gate.ingest(good, 20), false);
  snapshot = gate.snapshot(21);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);
  assert.equal(snapshot.attitude.ageMs, 20);
}

function testStatusOrderingIsIndependent() {
  const gate = ingress();
  gate.ingest(statusPacket(stamp(10, 1_000n), 0, 2), 0);
  assert.equal(gate.ingest(statusPacket(stamp(10, 1_000n), 0b1111, 0), 1), false);
  assert.equal(gate.ingest(statusPacket(stamp(9, 900n), 0b1111, 0), 2), false);
  assert.equal(gate.ingest(packet(stamp(1, 1_100n), null, 5), 3), true);

  const snapshot = gate.snapshot(3);
  assert.equal(snapshot.attitude.quat.w, 5);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);
  assert.equal(gate.diagnostics().duplicates, 1);
  assert.equal(gate.diagnostics().reordered, 1);
}

function testStatusResetsWithSourceIdentity() {
  const gate = new AvionicsIngress({
    vehicleId: VEHICLE,
    maximumSkewNanos: 100n,
    incarnationPolicy: INCARNATION_POLICY.SIM_ACCEPT_UNSEEN,
  });
  gate.ingest(statusPacket(stamp(1, 1_000n), 0b1111, 0), 0);
  gate.ingest(packet(stamp(0, 1n, 2), null, 2), 1);
  let snapshot = gate.snapshot(1);
  assert.equal(snapshot.estimatorStatus, null);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);

  gate.ingest(statusPacket(stamp(0, 2n, 2), 0b1111, 0), 2);
  gate.ingest(packet(stamp(0, 1n, 2, SOURCE, INCARNATION_B), null, 3), 3);
  snapshot = gate.snapshot(3);
  assert.equal(snapshot.sourceIncarnation, INCARNATION_B);
  assert.equal(snapshot.estimatorStatus, null);
  assert.equal(snapshot.validFlags, 0);
  assert.equal(snapshot.quality, 2);
}

function testSnapshotsAreAtomicAndImmutable() {
  const gate = ingress();
  const firstStatus = stamp(10, 10n);
  gate.ingest(statusPacket(firstStatus, 0b1111, 0), 0);
  gate.ingest(pairedPacket(stamp(1, 10n), stamp(1, 10n), firstStatus, 1, 0b1111, 0), 0);
  const before = gate.snapshot(0);
  gate.ingest(
    pairedPacket(stamp(2, 20n), stamp(2, 20n), stamp(11, 20n), 2, 0, 2),
    1,
  );
  const after = gate.snapshot(1);

  assert.equal(before.generation, 2);
  assert.equal(before.attitude.quat.w, 1);
  assert.equal(before.kinematics.posNed[0], 1);
  assert.equal(after.generation, 3);
  assert.equal(after.attitude.quat.w, 2);
  assert.equal(after.kinematics.posNed[0], 2);
  assert.equal(Object.isFrozen(before), true);
  assert.equal(Object.isFrozen(before.attitude.quat), true);
  assert.equal(Object.isFrozen(before.estimatorStatus), true);
  assert.equal(before.validFlags, 0b1111);
  assert.equal(after.validFlags, 0);
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
  testStatusOnlyRevocationDoesNotRefreshMeasurements,
  testStatusPayloadAloneControlsAuthorization,
  testMismatchedGroupCannotRegainThroughOtherNumeric,
  testStatusGoodReauthorizesOnlyExactNumericGroup,
  testCombinedStatusRevokesBeforeOneNumericGroupRecovers,
  testDuplicateStatusStampCanOnlyPublishALocalDowngrade,
  testStatusOrderingIsIndependent,
  testStatusResetsWithSourceIdentity,
  testSnapshotsAreAtomicAndImmutable,
  testInvalidStampIsRejected,
  testCountersAndGenerationWrap,
]) {
  test();
  console.log(`ok - ${test.name}`);
}

// ---- GEO-68: out-of-range / wrong-kind stamp identity rejected, typed reason -

{
  const gate = ingress();
  assert.equal(
    gate.ingest(packet(stamp(1, 100n, 0x1_0000_0000), null), 0),
    false,
    "a source_epoch past u32 is refused, never clamped into range",
  );
  assert.deepEqual(gate.diagnostics().lastRejectReason, {
    field: "sourceEpoch",
    rule: "out-of-range",
  });
}
{
  const gate = ingress();
  // sourceId is a u64: a Number is the wrong numeric kind (silent 2^53 truncation).
  assert.equal(gate.ingest(packet(stamp(1, 100n, 1, 10), null), 0), false);
  assert.deepEqual(gate.diagnostics().lastRejectReason, {
    field: "sourceId",
    rule: "wrong-numeric-kind",
  });
}
{
  const gate = ingress();
  assert.equal(
    gate.ingest(packet(stamp(1, 100n, 0xffff_ffff), null), 0),
    true,
    "the exact u32 max epoch is accepted",
  );
}
