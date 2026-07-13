// Behavioral checks for capture-to-aircraft-snapshot association (ADR-0020).
//
// Run: node clients/web/snapshot-association.test.mjs

import {
  SnapshotAssociator,
  associateIfAccepted,
  ASSOCIATION,
  DEFAULT_HISTORY_CAPACITY,
} from "./snapshot-association.js";
import { VideoIdentityTracker, DEFAULT_MAX_CLOCK_ERROR_NANOS } from "./video-identity.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

const INC_A = "aa".repeat(16);
const INC_B = "bb".repeat(16);
const CLOCK_VEHICLE_BOOT = 1;
const CLOCK_SIMULATION = 2;
const RECOGNIZED = { recognizedCalibrations: new Set([7]) };

// One accepted aircraft snapshot's AV-01 identity (kinematics-anchored). The
// snapshot clock is the flight-state clock (vehicle-boot) the video mapping
// targets.
function snapId(overrides = {}) {
  return {
    sourceId: 10n,
    sourceIncarnation: INC_A,
    sourceEpoch: 0,
    sequence: 0,
    acquiredAtNanos: 1000n,
    clock: CLOCK_VEHICLE_BOOT,
    ...overrides,
  };
}

// A parsed v2 video frame: capture on the sim clock, mapped into the vehicle-
// boot flight clock. Carries both the tracker-identity fields and the mapping
// fields, so it works with associate() and associateIfAccepted().
function frameMeta(overrides = {}) {
  return {
    sourceId: 0,
    sourceEpoch: 0,
    sourceIncarnation: INC_B,
    sequence: 0,
    captureTimeNanos: 1000n,
    captureClock: CLOCK_SIMULATION,
    cameraId: 0,
    calibrationId: 7,
    mappingAvailable: true,
    mappingTargetClock: CLOCK_VEHICLE_BOOT,
    mappingOffsetNanos: 0n,
    clockErrorBoundNanos: 0n,
    ...overrides,
  };
}

// Exact-match association: capture time maps onto a snapshot's acquisition time.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ sequence: 1, acquiredAtNanos: 1000n }));
  const v = a.associate(frameMeta({ captureTimeNanos: 1000n }), RECOGNIZED);
  check("exact match is ready", v.ready === true && v.reason === ASSOCIATION.READY);
  check("exact match selects the snapshot", v.snapshotIdentity.acquiredAtNanos === 1000n);
  check("exact match maps the capture time", v.mappedCaptureNanos === 1000n);
  check("exact match has zero total error", v.totalErrorNanos === 0n);
}

// Nearest-selection among several snapshots.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ sequence: 1, acquiredAtNanos: 1000n }));
  a.observe(snapId({ sequence: 2, acquiredAtNanos: 2000n }));
  a.observe(snapId({ sequence: 3, acquiredAtNanos: 3000n }));
  const v = a.associate(frameMeta({ captureTimeNanos: 2100n }), RECOGNIZED);
  check("nearest selection picks the closest snapshot", v.snapshotIdentity.sequence === 2);
  check("nearest selection reports the association delta as total error", v.totalErrorNanos === 100n);
  check("nearest selection is ready within budget", v.ready === true);
}

// Fail-closed: empty history.
{
  const v = new SnapshotAssociator().associate(frameMeta(), RECOGNIZED);
  check("empty history is not ready", v.ready === false && v.reason === ASSOCIATION.EMPTY_HISTORY);
}

// Fail-closed: no snapshot in the mapping's target clock domain.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ clock: CLOCK_VEHICLE_BOOT, acquiredAtNanos: 1000n }));
  const v = a.associate(frameMeta({ mappingTargetClock: CLOCK_SIMULATION }), RECOGNIZED);
  check("clock-domain mismatch is not ready", v.ready === false && v.reason === ASSOCIATION.CLOCK_MISMATCH);
}

// Fail-closed: nearest snapshot outside budget (total error = bound + delta).
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ acquiredAtNanos: 0n }));
  const v = a.associate(frameMeta({ captureTimeNanos: DEFAULT_MAX_CLOCK_ERROR_NANOS + 1000n }), RECOGNIZED);
  check("nearest outside budget is not ready", v.ready === false);
  check("over-budget reason is reported", v.reason === ASSOCIATION.TOTAL_ERROR_EXCEEDS_BUDGET);
  check("over-budget still reports the mapped time and total error", v.mappedCaptureNanos !== null && v.totalErrorNanos !== null);
}

// Total error combines the mapping's error bound with the association delta:
// each alone is within budget, together they exceed it.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ acquiredAtNanos: 1000n }));
  const bound = DEFAULT_MAX_CLOCK_ERROR_NANOS - 100n;
  const v = a.associate(
    frameMeta({ captureTimeNanos: 1200n, clockErrorBoundNanos: bound }),
    RECOGNIZED,
  );
  check("total error sums bound and delta", v.totalErrorNanos === bound + 200n);
  check("summed error over budget is not ready", v.ready === false && v.reason === ASSOCIATION.TOTAL_ERROR_EXCEEDS_BUDGET);
}

// Fail-closed: the nearest snapshot is from a superseded source incarnation.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ sourceIncarnation: INC_A, acquiredAtNanos: 1000n }));
  a.observe(snapId({ sourceIncarnation: INC_B, acquiredAtNanos: 5000n }));
  const v = a.associate(frameMeta({ captureTimeNanos: 1000n }), RECOGNIZED);
  check("incarnation discontinuity is not ready", v.ready === false);
  check("incarnation discontinuity reason is reported", v.reason === ASSOCIATION.STREAM_DISCONTINUITY);
}

// (a) Same source and incarnation, epoch increments (a reset within one
// attachment): a snapshot from the OLD epoch that is closer in time is not
// selected — the stream is identified by sourceId + incarnation + epoch.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ sourceEpoch: 0, acquiredAtNanos: 1000n }));
  a.observe(snapId({ sourceEpoch: 1, acquiredAtNanos: 5000n }));
  const v = a.associate(frameMeta({ captureTimeNanos: 1000n }), RECOGNIZED);
  check("epoch reset: old-epoch nearest snapshot is not selected", v.ready === false && v.reason === ASSOCIATION.STREAM_DISCONTINUITY);
  check("epoch reset: no old-epoch snapshot identity is returned", v.snapshotIdentity === null);
}

// (b) After a source switch, the old source is never selected even when nearest.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ sourceId: 10n, acquiredAtNanos: 1000n }));
  a.observe(snapId({ sourceId: 20n, acquiredAtNanos: 5000n }));
  const v = a.associate(frameMeta({ captureTimeNanos: 1000n }), RECOGNIZED);
  check("source switch: old source is never selected", v.ready === false && v.reason === ASSOCIATION.STREAM_DISCONTINUITY);
  check("source switch: no old-source snapshot identity is returned", v.snapshotIdentity === null);
}

// (c) Two identities differing ONLY in sourceId must not be deduped.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ sourceId: 10n, acquiredAtNanos: 1000n }));
  const second = a.observe(snapId({ sourceId: 20n, acquiredAtNanos: 1000n }));
  check("differing sourceId is not deduped", second === true);
  check("both distinct-source snapshots are retained", a.diagnostics().size === 2 && a.diagnostics().deduped === 0);
}

// (d) After a transport/session reset, the old session's history cannot produce
// a ready verdict.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ acquiredAtNanos: 1000n }));
  const before = a.associate(frameMeta({ captureTimeNanos: 1000n }), RECOGNIZED);
  check("pre-reset frame would be ready", before.ready === true);
  a.reset();
  const after = a.associate(frameMeta({ captureTimeNanos: 1000n }), RECOGNIZED);
  check("after session reset the old session yields no ready verdict", after.ready === false && after.reason === ASSOCIATION.EMPTY_HISTORY);
}

// Budget boundary is inclusive: a total error exactly at the budget is ready,
// one nanosecond over is not.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ acquiredAtNanos: 0n }));
  const atBudget = a.associate(
    frameMeta({ captureTimeNanos: DEFAULT_MAX_CLOCK_ERROR_NANOS }),
    RECOGNIZED,
  );
  check("total error exactly at budget is ready (inclusive)", atBudget.ready === true);
  check("at-budget total error is the budget", atBudget.totalErrorNanos === DEFAULT_MAX_CLOCK_ERROR_NANOS);

  const overBudget = a.associate(
    frameMeta({ captureTimeNanos: DEFAULT_MAX_CLOCK_ERROR_NANOS + 1n }),
    RECOGNIZED,
  );
  check("one nanosecond over budget is not ready", overBudget.ready === false);
}

// The ring is bounded: overflow drops the oldest entry and counts it.
{
  const a = new SnapshotAssociator({ capacity: 2 });
  a.observe(snapId({ sequence: 1, acquiredAtNanos: 1000n }));
  a.observe(snapId({ sequence: 2, acquiredAtNanos: 2000n }));
  a.observe(snapId({ sequence: 3, acquiredAtNanos: 3000n }));
  check("overflow drops the oldest, counted", a.diagnostics().dropped === 1);
  check("ring holds only the capacity", a.diagnostics().size === 2);
  const v = a.associate(frameMeta({ captureTimeNanos: 1000n }), RECOGNIZED);
  check("the dropped oldest snapshot is gone", v.snapshotIdentity.sequence !== 1);
}

// Consecutive identical snapshots are deduped (main.js re-reads the accepted
// snapshot each telemetry frame).
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ sequence: 1, acquiredAtNanos: 1000n }));
  const second = a.observe(snapId({ sequence: 1, acquiredAtNanos: 1000n }));
  check("consecutive identical snapshot is deduped", second === false && a.diagnostics().deduped === 1);
  check("dedup keeps one entry", a.diagnostics().size === 1);
}

// The association is finally passed through conformalGate: an unrecognized
// calibration closes the gate even when the snapshot distance and clock are fine.
{
  const a = new SnapshotAssociator();
  a.observe(snapId({ acquiredAtNanos: 1000n }));
  // No recognized-calibration option: the sim's default fail-closed set.
  const v = a.associate(frameMeta({ captureTimeNanos: 1000n, calibrationId: 7 }));
  check("unrecognized calibration closes the wrapped gate", v.ready === false);
  check("gate-closed reason is surfaced", v.reason === "gate-closed:calibration-unavailable");
  check("gate-closed still reports the associated snapshot", v.snapshotIdentity.acquiredAtNanos === 1000n);
}

// A malformed snapshot identity is rejected without entering the ring.
{
  const a = new SnapshotAssociator();
  check("malformed snapshot is not observed", a.observe(snapId({ sourceIncarnation: "nothex" })) === false);
  check("malformed snapshot is counted", a.diagnostics().invalid === 1 && a.diagnostics().size === 0);
}

// End-to-end: a tracker-accepted frame associates to the correct snapshot with a
// quantified total error.
{
  const tracker = new VideoIdentityTracker();
  const a = new SnapshotAssociator();
  a.observe(snapId({ sequence: 4, acquiredAtNanos: 1000n }));
  const meta = frameMeta({ sequence: 1, captureTimeNanos: 1000n, clockErrorBoundNanos: 100n });
  const res = associateIfAccepted(tracker, a, meta, RECOGNIZED);
  check("end-to-end: frame is admitted", res.accepted === true);
  check("end-to-end: association is ready", res.association.ready === true);
  check("end-to-end: correct snapshot identity", res.association.snapshotIdentity.acquiredAtNanos === 1000n);
  check("end-to-end: quantified total error (bound + delta)", res.association.totalErrorNanos === 100n);
}

// A replayed (duplicate) frame is rejected by the tracker, so it never produces
// a fresh association.
{
  const tracker = new VideoIdentityTracker();
  const a = new SnapshotAssociator();
  a.observe(snapId({ acquiredAtNanos: 1000n }));
  const first = associateIfAccepted(tracker, a, frameMeta({ sequence: 5, captureTimeNanos: 1000n }), RECOGNIZED);
  check("first frame is admitted and associated", first.accepted === true && first.association.ready === true);
  const replay = associateIfAccepted(tracker, a, frameMeta({ sequence: 5, captureTimeNanos: 1000n }), RECOGNIZED);
  check("replayed frame is not admitted", replay.accepted === false);
  check("replayed frame produces no fresh association", replay.association.ready === false && replay.association.reason === ASSOCIATION.NOT_ADMITTED);
}

check("default history capacity is documented and positive", Number.isInteger(DEFAULT_HISTORY_CAPACITY) && DEFAULT_HISTORY_CAPACITY > 0);

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall snapshot association checks passed");
