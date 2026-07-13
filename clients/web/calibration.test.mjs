// Fail-closed checks for the published simulator calibration (ADR-0021), and
// its effect on the HUD-01 conformal gate.
//
// Run: node clients/web/calibration.test.mjs

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import {
  loadCalibrationArtifact,
  CalibrationRegistry,
} from "./calibration.js";
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

// The published FPV calibration: id 1, camera 0, effective 2020..2035.
const FPV_CALIBRATION_ID = 1;
const FPV_CAMERA_ID = 0;
const NOW_IN_WINDOW = 1_600_000_000_000_000_000n; // ~2020-09, inside the window
const NOW_BEFORE_WINDOW = 1_000_000_000_000_000_000n; // ~2001, before the window
const CLOCK_VEHICLE_BOOT = 1;

// A v2 frame from the FPV camera, mapped to the vehicle-boot clock, that would
// be conformal-ready IF its calibration is recognized.
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

// The published artifact loads and hash-verifies.
{
  const loaded = await loadCalibrationArtifact(artifact);
  check("published artifact hash-verifies", loaded.ok === true);
  check("artifact carries the FPV calibration id", loaded.calibrationId === FPV_CALIBRATION_ID);
  check("artifact carries the FPV camera id", loaded.cameraId === FPV_CAMERA_ID);
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

// (4) EXPIRED (evaluation time outside the effective window) keeps the gate closed.
{
  const loaded = await loadCalibrationArtifact(artifact);
  const registry = new CalibrationRegistry().add(loaded);
  const recognized = registry.recognizedFor(FPV_CAMERA_ID, NOW_BEFORE_WINDOW);
  check("expired calibration is not recognized", recognized.size === 0);
  check("expired calibration keeps the gate closed", gateWith(recognized).conformalReady === false);
}

// (5) WRONG-CAMERA (frame from a different camera than the calibration) keeps
// the gate closed.
{
  const loaded = await loadCalibrationArtifact(artifact);
  const registry = new CalibrationRegistry().add(loaded);
  const recognized = registry.recognizedFor(FPV_CAMERA_ID + 1, NOW_IN_WINDOW);
  check("wrong-camera calibration is not recognized", recognized.size === 0);
  check("wrong-camera calibration keeps the gate closed", gateWith(recognized).conformalReady === false);
}

// When everything lines up, the calibration is recognized and the gate opens.
{
  const loaded = await loadCalibrationArtifact(artifact);
  const registry = new CalibrationRegistry().add(loaded);
  const recognized = registry.recognizedFor(FPV_CAMERA_ID, NOW_IN_WINDOW);
  check("aligned calibration is recognized", recognized.has(FPV_CALIBRATION_ID));
  check("aligned calibration opens the gate", gateWith(recognized).conformalReady === true);
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall calibration checks passed");
