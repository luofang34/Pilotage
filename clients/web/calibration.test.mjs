// Fail-closed checks for the published simulator calibration (ADR-0021): hash
// verification, semantic validation, the alignment budget, registry conflicts,
// and the effect on the HUD-01 conformal gate.
//
// Run: node clients/web/calibration.test.mjs

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { loadCalibrationArtifact, CalibrationRegistry } from "./calibration.js";
import { conformalGate } from "./video-identity.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

const here = dirname(fileURLToPath(import.meta.url));
const artifact = JSON.parse(readFileSync(join(here, "sim-fpv-calibration.json"), "utf8"));

const FPV_CALIBRATION_ID = 1;
const FPV_CAMERA_ID = 0;
const NOW_IN_WINDOW = 1_600_000_000_000_000_000n;
const NOW_BEFORE_WINDOW = 1_000_000_000_000_000_000n;
const CLOCK_VEHICLE_BOOT = 1;

function fpvFrameMeta(overrides = {}) {
  return {
    sourceId: 0,
    sourceEpoch: 0,
    sourceIncarnation: "ab".repeat(16),
    sequence: 0,
    captureTimeNanos: 1000n,
    captureClock: 2,
    cameraId: FPV_CAMERA_ID,
    calibrationId: FPV_CALIBRATION_ID,
    mappingAvailable: true,
    mappingTargetClock: CLOCK_VEHICLE_BOOT,
    mappingOffsetNanos: 0n,
    clockErrorBoundNanos: 0n,
    ...overrides,
  };
}
const snapshot = Object.freeze({ clock: CLOCK_VEHICLE_BOOT });
function gateWith(recognized) {
  return conformalGate(fpvFrameMeta(), snapshot, { recognizedCalibrations: recognized });
}

// ---- helpers to build hash-consistent but mutated artifacts ----------------
function base64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
  return bytes;
}
function bytesToBase64(bytes) {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}
async function sha256Hex(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}
// Mutate a fresh copy of the canonical bytes, then RE-RECORD its hash so the
// bytes and their recorded hash AGREE — the artifact is authentic but its
// geometry is semantically invalid.
async function mutated(apply) {
  const bytes = base64ToBytes(artifact.canonicalBase64);
  apply(new DataView(bytes.buffer, bytes.byteOffset, bytes.length), bytes);
  return { canonicalBase64: bytesToBase64(bytes), recordedHashHex: await sha256Hex(bytes) };
}

// The published artifact loads, hash-verifies, and passes semantic validation.
{
  const loaded = await loadCalibrationArtifact(artifact);
  check("published artifact loads and validates", loaded.ok === true);
  check("artifact carries the FPV calibration id", loaded.calibrationId === FPV_CALIBRATION_ID);
  check("artifact carries the FPV camera id", loaded.cameraId === FPV_CAMERA_ID);
  check("artifact exposes a positive alignment angular bound", loaded.totalAngularBoundRad > 0);
}

// (1) MISSING calibration keeps the gate closed.
{
  const loaded = await loadCalibrationArtifact(undefined);
  check("missing calibration does not load", loaded.ok === false && loaded.reason === "missing");
  const recognized = new CalibrationRegistry().add(loaded).recognizedFor(FPV_CAMERA_ID, NOW_IN_WINDOW);
  check("missing calibration keeps the gate closed", gateWith(recognized).conformalReady === false);
}

// (2) MISMATCHED (recorded hash does not match the bytes) keeps the gate closed.
{
  const tampered = { ...artifact, recordedHashHex: "00".repeat(32) };
  const loaded = await loadCalibrationArtifact(tampered);
  check("mismatched hash does not load", loaded.ok === false && loaded.reason === "hash-mismatch");
  const recognized = new CalibrationRegistry().add(loaded).recognizedFor(FPV_CAMERA_ID, NOW_IN_WINDOW);
  check("mismatched calibration keeps the gate closed", gateWith(recognized).conformalReady === false);
}

// (3) CORRUPT (truncated canonical bytes) keeps the gate closed.
{
  const corrupt = { canonicalBase64: "AAAA", recordedHashHex: artifact.recordedHashHex };
  const loaded = await loadCalibrationArtifact(corrupt);
  check("corrupt calibration does not load", loaded.ok === false && loaded.reason === "corrupt");
  const recognized = new CalibrationRegistry().add(loaded).recognizedFor(FPV_CAMERA_ID, NOW_IN_WINDOW);
  check("corrupt calibration keeps the gate closed", gateWith(recognized).conformalReady === false);
}

// (4) EXPIRED keeps the gate closed.
{
  const registry = new CalibrationRegistry().add(await loadCalibrationArtifact(artifact));
  const recognized = registry.recognizedFor(FPV_CAMERA_ID, NOW_BEFORE_WINDOW);
  check("expired calibration is not recognized", recognized.size === 0);
  check("expired calibration keeps the gate closed", gateWith(recognized).conformalReady === false);
}

// (5) WRONG-CAMERA keeps the gate closed.
{
  const registry = new CalibrationRegistry().add(await loadCalibrationArtifact(artifact));
  const recognized = registry.recognizedFor(FPV_CAMERA_ID + 1, NOW_IN_WINDOW);
  check("wrong-camera calibration is not recognized", recognized.size === 0);
  check("wrong-camera calibration keeps the gate closed", gateWith(recognized).conformalReady === false);
}

// ---- semantic validation: hash-consistent but invalid geometry -------------
// Each artifact's bytes and recorded hash agree, yet a semantic invariant is
// violated; the browser admission must reject it (hash integrity != validity).
const semanticCases = [
  ["non-finite focal", (v) => v.setFloat64(53, NaN, true), "invalid:non-finite"],
  ["zero viewport", (v) => v.setUint32(45, 0, true), "invalid:invalid-viewport"],
  ["out-of-range FOV", (v) => v.setFloat64(133, 4.0, true), "invalid:invalid-fov"],
  ["non-positive focal", (v) => v.setFloat64(53, -1.0, true), "invalid:non-positive-focal"],
  ["principal point out of bounds", (v) => v.setFloat64(69, 1e4, true), "invalid:principal-point-out-of-bounds"],
  ["wrong extrinsic frame", (v) => v.setUint8(43, 2), "invalid:frame-mismatch"],
  ["non-unit quaternion", (v) => v.setFloat64(173, 2.0, true), "invalid:non-unit-quaternion"],
  ["non-unit boresight", (v) => v.setFloat64(229, 2.0, true), "invalid:non-unit-boresight"],
  ["inverted effective period", (v) => v.setBigUint64(28, v.getBigUint64(20, true), true), "invalid:invalid-effective-period"],
  ["negative residuals", (v) => v.setFloat64(253, -1.0, true), "invalid:invalid-residuals"],
  ["inconsistent alignment budget", (v) => v.setFloat64(325, 999.0, true), "invalid:invalid-budget"],
];
for (const [name, apply, reason] of semanticCases) {
  const loaded = await loadCalibrationArtifact(await mutated(apply));
  check(`semantic reject: ${name}`, loaded.ok === false && loaded.reason === reason);
  const recognized = new CalibrationRegistry().add(loaded).recognizedFor(FPV_CAMERA_ID, NOW_IN_WINDOW);
  check(`semantic reject keeps the gate closed: ${name}`, gateWith(recognized).conformalReady === false);
}

// ---- alignment error budget surfaced through admission ---------------------
{
  const registry = new CalibrationRegistry().add(await loadCalibrationArtifact(artifact));
  const bound = registry.alignmentBoundFor(FPV_CALIBRATION_ID);
  check("alignment bound is surfaced with provenance", bound !== null && bound.angularBoundRad > 0);
  check(
    "alignment bound carries the recorded hash as provenance",
    bound.recordedHashHex === artifact.recordedHashHex,
  );
  check("alignment bound is conservative (< 0.05 rad for the sim FPV)", bound.angularBoundRad < 0.05);
}

// ---- registry conflict: same id, different content, NEITHER admitted -------
{
  const genuine = await loadCalibrationArtifact(artifact);
  // A different-content artifact reusing the same calibration id (a bumped
  // version → different bytes → different hash).
  const conflicting = await loadCalibrationArtifact(
    await mutated((v) => v.setUint32(10, 2, true)), // version 1 -> 2, same id
  );
  const registry = new CalibrationRegistry().add(genuine).add(conflicting);
  check("a conflicting id is recorded as conflicted", registry.conflicts().has(FPV_CALIBRATION_ID));
  const recognized = registry.recognizedFor(FPV_CAMERA_ID, NOW_IN_WINDOW);
  check("a conflicting id is not recognized (neither admitted)", recognized.size === 0);
  check("a conflicting id keeps the gate closed", gateWith(recognized).conformalReady === false);
  check("a conflicting id has no alignment bound", registry.alignmentBoundFor(FPV_CALIBRATION_ID) === null);
}

// An exact duplicate (same id, same hash) is deduped, not a conflict.
{
  const registry = new CalibrationRegistry()
    .add(await loadCalibrationArtifact(artifact))
    .add(await loadCalibrationArtifact(artifact));
  check("exact duplicate is not a conflict", registry.conflicts().size === 0);
  check(
    "exact duplicate still recognized",
    registry.recognizedFor(FPV_CAMERA_ID, NOW_IN_WINDOW).has(FPV_CALIBRATION_ID),
  );
}

// When everything lines up, the calibration is recognized and the gate opens.
{
  const registry = new CalibrationRegistry().add(await loadCalibrationArtifact(artifact));
  const recognized = registry.recognizedFor(FPV_CAMERA_ID, NOW_IN_WINDOW);
  check("aligned calibration is recognized", recognized.has(FPV_CALIBRATION_ID));
  check("aligned calibration opens the gate", gateWith(recognized).conformalReady === true);
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall calibration checks passed");
