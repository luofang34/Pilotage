// Behavioral checks for the video capture-identity tracker and conformal gate
// (ADR-0020).
//
// Run: node clients/web/video-identity.test.mjs

import {
  VideoIdentityTracker,
  conformalGate,
  ADMIT,
  DEFAULT_MAX_CLOCK_ERROR_NANOS,
} from "./video-identity.js";
import {
  SnapshotAssociator,
  associateIfAccepted,
  ASSOCIATION,
} from "./snapshot-association.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

const INCARNATION_A = "ab".repeat(16);
const INCARNATION_B = "cd".repeat(16);

function meta(overrides = {}) {
  return {
    sourceId: 0,
    sourceEpoch: 0,
    sourceIncarnation: INCARNATION_A,
    sequence: 0,
    captureTimeNanos: 1000n,
    captureClock: 2,
    mappingAvailable: true,
    mappingTargetClock: 2,
    mappingOffsetNanos: 0n,
    clockErrorBoundNanos: 0n,
    receiveTimeNanos: 0n,
    publicationTimeNanos: 0n,
    cameraId: 0,
    calibrationId: 0,
    ...overrides,
  };
}

// First frame establishes the source and is a discontinuity (fresh start).
{
  const t = new VideoIdentityTracker();
  const first = t.admit(meta({ sequence: 5, captureTimeNanos: 5000n }));
  check("first frame is accepted", first.accepted && first.reason === ADMIT.ACCEPTED);
  check("first frame marks a discontinuity", first.discontinuity === true);
}

// A strictly newer sequence advances; the accepted capture time follows it.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ sequence: 5, captureTimeNanos: 5000n }));
  const next = t.admit(meta({ sequence: 6, captureTimeNanos: 6000n }));
  check("newer sequence is accepted", next.accepted && next.discontinuity === false);
  check(
    "accepted state tracks the newest frame's capture time",
    t.lastAccepted(0).captureTimeNanos === 6000n,
  );
}

// A duplicate sequence is dropped and does NOT refresh the frame's age: the
// last accepted capture time stays the earlier value even though the replay
// carried a later one.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ sequence: 5, captureTimeNanos: 5000n }));
  const replay = t.admit(meta({ sequence: 5, captureTimeNanos: 9999n }));
  check("duplicate sequence is not accepted", replay.accepted === false);
  check("duplicate reason is reported", replay.reason === ADMIT.DUPLICATE);
  check(
    "duplicate does not refresh the accepted capture time",
    t.lastAccepted(0).captureTimeNanos === 5000n,
  );
  check("duplicate is counted", t.diagnostics().duplicates === 1);
}

// A reordered (older) sequence is dropped and leaves state untouched.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ sequence: 5, captureTimeNanos: 5000n }));
  const older = t.admit(meta({ sequence: 3, captureTimeNanos: 3000n }));
  check("older sequence is not accepted", older.accepted === false);
  check("older sequence reason is reordered", older.reason === ADMIT.REORDERED);
  check(
    "reordered frame does not move the accepted sequence",
    t.lastAccepted(0).sequence === 5,
  );
}

// The wrapping sequence is compared with serial arithmetic: 0 is newer than
// 0xFFFFFFFF, so a frame straddling the u32 boundary advances rather than being
// mistaken for an ancient reorder.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ sequence: 0xffffffff, captureTimeNanos: 5000n }));
  const wrapped = t.admit(meta({ sequence: 0, captureTimeNanos: 6000n }));
  check("sequence wrap (MAX -> 0) is accepted as newer", wrapped.accepted === true);
  check("wrapped frame advances the accepted sequence", t.lastAccepted(0).sequence === 0);
  // A backward wrap of the same magnitude is still a reorder.
  const back = t.admit(meta({ sequence: 0xffffffff, captureTimeNanos: 7000n }));
  check("wrapping backward is still rejected", back.accepted === false);
}

// A newer epoch resets the timeline: it is accepted, flagged as a
// discontinuity, and lets a lower sequence through under the new epoch.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ sourceEpoch: 0, sequence: 9, captureTimeNanos: 5000n }));
  const reset = t.admit(meta({ sourceEpoch: 1, sequence: 0, captureTimeNanos: 6000n }));
  check("newer epoch is accepted", reset.accepted === true);
  check("newer epoch marks a discontinuity", reset.discontinuity === true);
  check("epoch reset is counted", t.diagnostics().epochResets === 1);
  check("epoch reset re-bases the sequence", t.lastAccepted(0).epoch === 1 && t.lastAccepted(0).sequence === 0);
}

// An older epoch is stale and dropped.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ sourceEpoch: 5, sequence: 0 }));
  const stale = t.admit(meta({ sourceEpoch: 4, sequence: 100 }));
  check("older epoch is not accepted", stale.accepted === false);
  check("older epoch reason is stale-epoch", stale.reason === ADMIT.STALE_EPOCH);
  check("stale epoch is counted", t.diagnostics().staleEpochs === 1);
}

// A frame whose camera id differs from the source's established camera is
// rejected rather than mixed into the same stream.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ cameraId: 0, sequence: 0 }));
  const wrong = t.admit(meta({ cameraId: 1, sequence: 1 }));
  check("wrong camera id is not accepted", wrong.accepted === false);
  check("wrong camera reason is reported", wrong.reason === ADMIT.WRONG_CAMERA);
  check("wrong camera is counted", t.diagnostics().wrongCamera === 1);
}

// A new incarnation (a fresh attachment) is a discontinuity that re-establishes
// the source.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ sourceIncarnation: INCARNATION_A, sequence: 9 }));
  const reattach = t.admit(meta({ sourceIncarnation: INCARNATION_B, sequence: 0 }));
  check("new incarnation is accepted", reattach.accepted === true);
  check("new incarnation marks a discontinuity", reattach.discontinuity === true);
  check("incarnation reset is counted", t.diagnostics().incarnationResets === 1);
}

// Sources are tracked independently: chase frames never affect FPV state.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ sourceId: 0, sequence: 5 }));
  const chase = t.admit(meta({ sourceId: 1, sequence: 0 }));
  check("a second source starts its own timeline", chase.accepted === true);
  check("first source keeps its own sequence", t.lastAccepted(0).sequence === 5);
  check("second source has its own sequence", t.lastAccepted(1).sequence === 0);
}

// A malformed stamp is rejected without disturbing state.
{
  const t = new VideoIdentityTracker();
  const bad = t.admit(meta({ sourceIncarnation: "nothex" }));
  check("malformed stamp is not accepted", bad.accepted === false);
  check("malformed reason is reported", bad.reason === ADMIT.MALFORMED);
}

// A calibration-ID change re-bases the camera model, so it must be an explicit
// discontinuity rather than silently continuing the conformal timeline.
{
  const t = new VideoIdentityTracker();
  t.admit(meta({ calibrationId: 5, sequence: 9 }));
  const recal = t.admit(meta({ calibrationId: 6, sequence: 0 }));
  check("calibration change is accepted", recal.accepted === true);
  check("calibration change marks a discontinuity", recal.discontinuity === true);
  check("calibration reset is counted", t.diagnostics().calibrationResets === 1);
  check("calibration change re-bases the tracked calibration", t.lastAccepted(0).calibrationId === 6);
}

// ---- conformal gate --------------------------------------------------------
// The gate consumes BOTH the frame metadata and the candidate aircraft snapshot
// identity (an AV-01 MeasurementStamp; only its clock is read here). It fails
// closed and demands: available mapping, a target clock matching the snapshot,
// error within budget, an overflow-free mapped time, and a published/recognized
// calibration.
const snap = (clock) => Object.freeze({ clock });
const RECOGNIZED = { recognizedCalibrations: new Set([7]) };
const CLOCK_SIMULATION = 2;
const CLOCK_VEHICLE_BOOT = 1;

// A calibration ID of zero (unpublished / CalibrationId::NONE) keeps the gate
// closed even when the clock side is fully valid.
{
  const g = conformalGate(
    meta({ mappingAvailable: true, mappingTargetClock: CLOCK_SIMULATION, calibrationId: 0 }),
    snap(CLOCK_SIMULATION),
    RECOGNIZED,
  );
  check("calibration id zero: clock side is valid", g.mappingValid === true);
  check("calibration id zero keeps the gate closed", g.conformalReady === false);
}

// A published but unrecognized (wrong/stale) calibration keeps the gate closed.
{
  const g = conformalGate(
    meta({ mappingAvailable: true, mappingTargetClock: CLOCK_SIMULATION, calibrationId: 99 }),
    snap(CLOCK_SIMULATION),
    RECOGNIZED,
  );
  check("unrecognized calibration keeps the gate closed", g.conformalReady === false);
}

// A mapping targeting a clock other than the candidate snapshot's is not usable.
{
  const g = conformalGate(
    meta({ mappingAvailable: true, mappingTargetClock: CLOCK_VEHICLE_BOOT, calibrationId: 7 }),
    snap(CLOCK_SIMULATION),
    RECOGNIZED,
  );
  check("target-clock mismatch keeps the gate closed", g.conformalReady === false);
  check("target-clock mismatch is not mapping-valid", g.mappingValid === false);
}

// An available mapping whose quantified error exceeds the budget keeps the gate
// closed: "bounded" alone is not sufficient.
{
  const overBudget = DEFAULT_MAX_CLOCK_ERROR_NANOS + 1n;
  const g = conformalGate(
    meta({
      mappingAvailable: true,
      mappingTargetClock: CLOCK_SIMULATION,
      calibrationId: 7,
      clockErrorBoundNanos: overBudget,
    }),
    snap(CLOCK_SIMULATION),
    RECOGNIZED,
  );
  check("excessive error bound keeps the gate closed", g.conformalReady === false);
  check("excessive error bound is not mapping-valid", g.mappingValid === false);
}

// A mapping whose signed offset would carry the capture time outside the u64
// range refuses rather than wrapping into a plausible time.
{
  const g = conformalGate(
    meta({
      mappingAvailable: true,
      mappingTargetClock: CLOCK_SIMULATION,
      calibrationId: 7,
      captureTimeNanos: 5n,
      mappingOffsetNanos: -6n,
    }),
    snap(CLOCK_SIMULATION),
    RECOGNIZED,
  );
  check("mapped-time underflow refuses", g.conformalReady === false && g.reason === "mapped-time-overflow");
}

// Only a fully compatible snapshot / calibration / mapping combination is ready.
{
  const g = conformalGate(
    meta({
      mappingAvailable: true,
      mappingTargetClock: CLOCK_SIMULATION,
      calibrationId: 7,
      captureTimeNanos: 1000n,
      mappingOffsetNanos: 50n,
      clockErrorBoundNanos: 250n,
    }),
    snap(CLOCK_SIMULATION),
    RECOGNIZED,
  );
  check("compatible combination is conformal-ready", g.conformalReady === true);
  check("ready verdict is mapping-valid", g.mappingValid === true);
  check("ready verdict carries the error bound", g.clockErrorBoundNanos === 250n);
  check("ready verdict exposes the mapped capture time", g.mappedCaptureTimeNanos === 1050n);
}

// An unavailable mapping is not mapping-valid regardless of the snapshot.
{
  const g = conformalGate(meta({ mappingAvailable: false }), snap(CLOCK_SIMULATION), RECOGNIZED);
  check("unavailable mapping is not mapping-valid", g.mappingValid === false);
  check("unavailable mapping is not conformal-ready", g.conformalReady === false);
  check("unavailable mapping has no error bound", g.clockErrorBoundNanos === null);
}

// The gate fails closed for absent metadata or an absent candidate snapshot.
check("gate on null meta is closed", conformalGate(null, snap(CLOCK_SIMULATION)).conformalReady === false);
check(
  "gate with no candidate snapshot is closed",
  conformalGate(meta({ mappingAvailable: true, calibrationId: 7 }), null, RECOGNIZED).conformalReady === false,
);

// The tracker verdict gates association: a frame the tracker rejects (here a
// stale epoch) never reaches the associator, so it cannot associate fresh.
{
  const tracker = new VideoIdentityTracker();
  const associator = new SnapshotAssociator();
  associator.observe({
    sourceId: 10n,
    sourceIncarnation: "aa".repeat(16),
    sourceEpoch: 0,
    sequence: 0,
    acquiredAtNanos: 1000n,
    clock: CLOCK_SIMULATION,
  });
  tracker.admit(meta({ sourceEpoch: 5, sequence: 0 }));
  const stale = associateIfAccepted(
    tracker,
    associator,
    meta({ sourceEpoch: 4, sequence: 9, mappingTargetClock: CLOCK_SIMULATION }),
    RECOGNIZED,
  );
  check("tracker-rejected frame is not admitted", stale.accepted === false);
  check("tracker-rejected frame does not associate", stale.association.reason === ASSOCIATION.NOT_ADMITTED);
}


// ---- GEO-68: hostile wire-range meta is refused as malformed ----------------

{
  const hostile = [
    ["negative sourceId (u8)", { sourceId: -1 }],
    ["overflowing sourceId (u8)", { sourceId: 256 }],
    ["bigint sourceId (u8 must be Number)", { sourceId: 0n }],
    ["overflowing sourceEpoch", { sourceEpoch: 0x1_0000_0000 }],
    ["fractional sequence", { sequence: 2.5 }],
    ["Number captureTimeNanos (u64 must be BigInt)", { captureTimeNanos: 1000 }],
    ["overflowing captureTimeNanos", { captureTimeNanos: (1n << 64n) }],
    ["negative cameraId", { cameraId: -1 }],
    ["overflowing calibrationId", { calibrationId: 0x1_0000_0000 }],
    ["bigint calibrationId (u32 must be Number)", { calibrationId: 7n }],
    ["malformed incarnation", { sourceIncarnation: "nope" }],
  ];
  for (const [name, override] of hostile) {
    const t = new VideoIdentityTracker();
    const verdict = t.admit(meta(override));
    check(
      `hostile meta refused as malformed: ${name}`,
      verdict.accepted === false && verdict.reason === ADMIT.MALFORMED,
    );
  }
}

// ---- GEO-68: admit() surfaces the typed { field, rule } reason --------------
// The rejection is not merely "malformed": the verdict and the tracker's
// diagnostics both name which wire field failed and why, across the identity
// tuple, the signed mapping offset, and the u64 error/receive/publication times.
{
  const cases = [
    [{ sourceId: 256 }, "sourceId", "out-of-range"],
    [{ calibrationId: 7n }, "calibrationId", "wrong-numeric-kind"],
    [{ captureTimeNanos: 1000 }, "captureTimeNanos", "wrong-numeric-kind"],
    [{ clockErrorBoundNanos: -1n }, "clockErrorBoundNanos", "negative"],
    [{ mappingOffsetNanos: 1n << 63n }, "mappingOffsetNanos", "out-of-range"],
    [{ receiveTimeNanos: 5 }, "receiveTimeNanos", "wrong-numeric-kind"],
    [{ sourceIncarnation: "nope" }, "sourceIncarnation", "malformed"],
  ];
  for (const [override, field, rule] of cases) {
    const t = new VideoIdentityTracker();
    const verdict = t.admit(meta(override));
    check(
      `admit reports the typed fault ${field}/${rule}`,
      verdict.accepted === false &&
        verdict.fault !== null &&
        verdict.fault.field === field &&
        verdict.fault.rule === rule,
    );
    check(
      `diagnostics exposes the last malformed reason ${field}/${rule}`,
      t.diagnostics().lastMalformedReason !== null &&
        t.diagnostics().lastMalformedReason.field === field &&
        t.diagnostics().lastMalformedReason.rule === rule,
    );
  }
}

// ---- GEO-68: a malformed meta identity can never be conformal-ready ---------

{
  const g = conformalGate(
    meta({ mappingAvailable: true, mappingTargetClock: CLOCK_SIMULATION, sourceId: 256, calibrationId: 7 }),
    snap(CLOCK_SIMULATION),
    RECOGNIZED,
  );
  check(
    "a malformed meta (u8 sourceId=256) is refused by the gate, never ready, with a typed reason",
    g.conformalReady === false && g.reason === "malformed-meta:sourceId:out-of-range",
  );
}
{
  const g = conformalGate(
    meta({ mappingAvailable: true, mappingTargetClock: CLOCK_SIMULATION, calibrationId: 7n }),
    snap(CLOCK_SIMULATION),
    RECOGNIZED,
  );
  check(
    "a bigint calibrationId (u32 must be Number) is refused by the gate with a typed reason",
    g.conformalReady === false && g.reason === "malformed-meta:calibrationId:wrong-numeric-kind",
  );
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall video identity checks passed");
