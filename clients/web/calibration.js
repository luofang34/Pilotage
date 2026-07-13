// Published simulator camera calibration: load, hash-verify, semantically
// validate, and expose the calibration ids recognized for a given frame plus
// the calibration's alignment error contribution (ADR-0021).
//
// SIM / NOT FOR FLIGHT. The artifact describes a SIMULATED pinhole camera and a
// synthetic design eye; it is never real HUD optics and must not be read as
// optical qualification. The published artifact carries the exact canonical
// bytes the recorded SHA-256 was taken over (mirroring
// pilotage_adapter_api::calibration::canonical). This module recomputes the
// hash over those bytes, then independently validates every geometry, lifecycle,
// and budget invariant the Rust `validate` checks — hash integrity is not
// semantic validity. A missing, mismatched, corrupt, semantically invalid,
// expired, wrong-camera, or conflicting calibration yields no recognized id, so
// the HUD-01 conformal gate stays closed.

// Mirrors CALIBRATION_SCHEMA_VERSION in the Rust canonical module.
export const CALIBRATION_SCHEMA_VERSION = 1;
// ValidityStatus::Valid.
const STATUS_VALID = 1;
// FrameId::Body / FrameId::Installation.
const FRAME_BODY = 0;
const FRAME_INSTALLATION = 1;
// Tolerances mirroring the Rust validator.
const UNIT_NORM_TOLERANCE = 1e-6;
const BUDGET_CONSISTENCY_TOLERANCE = 1e-9;
// Total canonical length (header + geometry + budget); the fixed layout.
const CALIBRATION_TOTAL_LEN = 333;

// Canonical byte offsets (little-endian), mirroring the Rust serialization.
const OFF = Object.freeze({
  schema: 0,
  calibrationId: 2,
  cameraId: 6,
  version: 10,
  effectiveStart: 20,
  effectiveEnd: 28,
  status: 40,
  fromFrame: 43,
  toFrame: 44,
  viewportWidth: 45,
  viewportHeight: 49,
  focalX: 53,
  focalY: 61,
  principalX: 69,
  principalY: 77,
  skew: 85,
  distortion: 93, // 5 x f64
  fovH: 133,
  fovV: 141,
  translation: 149, // 3 x f64
  quat: 173, // 4 x f64
  designEye: 205, // 3 x f64
  boresight: 229, // 3 x f64
  residualRms: 253,
  residualMax: 261,
  budgetIntrinsicResidual: 269,
  budgetDistortionAllowance: 277,
  budgetExtrinsicsAllowance: 285,
  budgetBoresightAllowance: 293,
  budgetDesignEyeAllowance: 301,
  budgetRadiansPerPixel: 309,
  budgetTotalPixel: 317,
  budgetTotalAngular: 325,
});

function base64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

function bytesToHex(bytes) {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

async function sha256Hex(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return bytesToHex(new Uint8Array(digest));
}

function f64s(view, offset, count) {
  const out = [];
  for (let i = 0; i < count; i += 1) out.push(view.getFloat64(offset + i * 8, true));
  return out;
}

function allFinite(values) {
  return values.every((v) => Number.isFinite(v));
}

function norm(values) {
  return Math.sqrt(values.reduce((sum, v) => sum + v * v, 0));
}

// Mirrors the Rust `validate` geometry checks: returns null when valid, else a
// typed reason.
function validateGeometry(view) {
  const width = view.getUint32(OFF.viewportWidth, true);
  const height = view.getUint32(OFF.viewportHeight, true);
  const focalX = view.getFloat64(OFF.focalX, true);
  const focalY = view.getFloat64(OFF.focalY, true);
  const principalX = view.getFloat64(OFF.principalX, true);
  const principalY = view.getFloat64(OFF.principalY, true);
  const intrinsics = [focalX, focalY, principalX, principalY, view.getFloat64(OFF.skew, true)];
  if (!allFinite(intrinsics) || !allFinite(f64s(view, OFF.distortion, 5))) return "non-finite";
  if (width === 0 || height === 0) return "invalid-viewport";
  const fovH = view.getFloat64(OFF.fovH, true);
  const fovV = view.getFloat64(OFF.fovV, true);
  if (!allFinite([fovH, fovV])) return "non-finite";
  if (fovH <= 0 || fovH >= Math.PI || fovV <= 0 || fovV >= Math.PI) return "invalid-fov";
  if (focalX <= 0 || focalY <= 0) return "non-positive-focal";
  if (principalX < 0 || principalX > width || principalY < 0 || principalY > height) {
    return "principal-point-out-of-bounds";
  }
  const quat = f64s(view, OFF.quat, 4);
  const boresight = f64s(view, OFF.boresight, 3);
  if (
    !allFinite(f64s(view, OFF.translation, 3)) ||
    !allFinite(quat) ||
    !allFinite(f64s(view, OFF.designEye, 3)) ||
    !allFinite(boresight)
  ) {
    return "non-finite";
  }
  if (
    view.getUint8(OFF.fromFrame) !== FRAME_BODY ||
    view.getUint8(OFF.toFrame) !== FRAME_INSTALLATION
  ) {
    return "frame-mismatch";
  }
  if (Math.abs(norm(quat) - 1) > UNIT_NORM_TOLERANCE) return "non-unit-quaternion";
  if (Math.abs(norm(boresight) - 1) > UNIT_NORM_TOLERANCE) return "non-unit-boresight";
  return null;
}

// Mirrors the Rust `validate` lifecycle and budget checks.
function validateLifecycleAndBudget(view) {
  if (view.getBigUint64(OFF.effectiveStart, true) >= view.getBigUint64(OFF.effectiveEnd, true)) {
    return "invalid-effective-period";
  }
  const rms = view.getFloat64(OFF.residualRms, true);
  const max = view.getFloat64(OFF.residualMax, true);
  if (!allFinite([rms, max]) || rms < 0 || max < 0 || rms > max) return "invalid-residuals";
  const intrinsicResidual = view.getFloat64(OFF.budgetIntrinsicResidual, true);
  const distortionAllowance = view.getFloat64(OFF.budgetDistortionAllowance, true);
  const extrinsicsAllowance = view.getFloat64(OFF.budgetExtrinsicsAllowance, true);
  const boresightAllowance = view.getFloat64(OFF.budgetBoresightAllowance, true);
  const designEyeAllowance = view.getFloat64(OFF.budgetDesignEyeAllowance, true);
  const radiansPerPixel = view.getFloat64(OFF.budgetRadiansPerPixel, true);
  const totalPixel = view.getFloat64(OFF.budgetTotalPixel, true);
  const totalAngular = view.getFloat64(OFF.budgetTotalAngular, true);
  const budget = [
    intrinsicResidual, distortionAllowance, extrinsicsAllowance, boresightAllowance,
    designEyeAllowance, radiansPerPixel, totalPixel, totalAngular,
  ];
  if (!allFinite(budget) || budget.some((v) => v < 0) || radiansPerPixel <= 0) return "invalid-budget";
  if (Math.abs(totalPixel - (intrinsicResidual + distortionAllowance)) > BUDGET_CONSISTENCY_TOLERANCE) {
    return "invalid-budget";
  }
  const expectedAngular =
    totalPixel * radiansPerPixel + extrinsicsAllowance + boresightAllowance + designEyeAllowance;
  if (Math.abs(totalAngular - expectedAngular) > BUDGET_CONSISTENCY_TOLERANCE) return "invalid-budget";
  return null;
}

/**
 * Loads, hash-verifies, and semantically validates one published calibration
 * artifact `{ canonicalBase64, recordedHashHex }`. Returns `{ ok: true, ... }`
 * with the parsed header fields and the alignment angular bound, or
 * `{ ok: false, reason }` for a missing, corrupt, hash-mismatched, wrong-schema,
 * or semantically invalid artifact. Fails closed throughout.
 */
export async function loadCalibrationArtifact(artifact) {
  if (
    !artifact ||
    typeof artifact.canonicalBase64 !== "string" ||
    typeof artifact.recordedHashHex !== "string"
  ) {
    return { ok: false, reason: "missing" };
  }
  let bytes;
  try {
    bytes = base64ToBytes(artifact.canonicalBase64);
  } catch {
    return { ok: false, reason: "corrupt" };
  }
  if (bytes.length !== CALIBRATION_TOTAL_LEN) return { ok: false, reason: "corrupt" };
  const computed = await sha256Hex(bytes);
  if (computed !== artifact.recordedHashHex) return { ok: false, reason: "hash-mismatch" };
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
  if (view.getUint16(OFF.schema, true) !== CALIBRATION_SCHEMA_VERSION) {
    return { ok: false, reason: "schema-mismatch" };
  }
  const geometryReason = validateGeometry(view);
  if (geometryReason) return { ok: false, reason: `invalid:${geometryReason}` };
  const lifecycleReason = validateLifecycleAndBudget(view);
  if (lifecycleReason) return { ok: false, reason: `invalid:${lifecycleReason}` };
  return {
    ok: true,
    calibrationId: view.getUint32(OFF.calibrationId, true),
    cameraId: view.getUint32(OFF.cameraId, true),
    version: view.getUint32(OFF.version, true),
    effectiveStartNs: view.getBigUint64(OFF.effectiveStart, true),
    effectiveEndNs: view.getBigUint64(OFF.effectiveEnd, true),
    status: view.getUint8(OFF.status),
    totalAngularBoundRad: view.getFloat64(OFF.budgetTotalAngular, true),
    recordedHashHex: artifact.recordedHashHex,
  };
}

/**
 * A set of hash-verified, semantically valid calibrations. Duplicate ids with
 * DIFFERENT content (a conflict) are rejected outright: neither definition is
 * admitted, so a consumer never resolves an ambiguous id. Exact duplicates
 * (same id, same recorded hash) are deduped, not conflicts.
 */
export class CalibrationRegistry {
  constructor() {
    this.byId = new Map();
    this.conflicted = new Set();
  }

  /** Adds a loaded calibration; a failed load (`ok: false`) is ignored. */
  add(loaded) {
    if (!loaded || loaded.ok !== true) return this;
    const existing = this.byId.get(loaded.calibrationId);
    if (existing && existing.recordedHashHex !== loaded.recordedHashHex) {
      this.conflicted.add(loaded.calibrationId);
    } else if (!existing) {
      this.byId.set(loaded.calibrationId, loaded);
    }
    return this;
  }

  recognizedFor(cameraId, nowUnixNs) {
    const recognized = new Set();
    for (const [id, cal] of this.byId) {
      if (this.conflicted.has(id)) continue;
      if (cal.status !== STATUS_VALID) continue;
      if (!(nowUnixNs >= cal.effectiveStartNs && nowUnixNs < cal.effectiveEndNs)) continue;
      if (cal.cameraId !== cameraId) continue;
      recognized.add(id);
    }
    return recognized;
  }

  /** The calibration's alignment-budget angular bound (radians) with its
   *  provenance, or `null` if the id is unknown or conflicting. One number a
   *  downstream budget composes. */
  alignmentBoundFor(calibrationId) {
    if (this.conflicted.has(calibrationId)) return null;
    const cal = this.byId.get(calibrationId);
    if (!cal) return null;
    return Object.freeze({
      calibrationId,
      angularBoundRad: cal.totalAngularBoundRad,
      recordedHashHex: cal.recordedHashHex,
    });
  }

  /** Ids that appeared with conflicting definitions, for diagnostics. */
  conflicts() {
    return new Set(this.conflicted);
  }
}

/** Fetches and hash-verifies the published artifact at `url`, returning a
 *  populated [`CalibrationRegistry`]. On any fetch or verification failure the
 *  registry is empty (fail-closed): conformal output simply stays off. */
export async function loadCalibrationRegistry(url) {
  const registry = new CalibrationRegistry();
  try {
    const response = await fetch(url);
    if (!response.ok) return registry;
    const artifact = await response.json();
    registry.add(await loadCalibrationArtifact(artifact));
  } catch {
    // Missing or unreadable artifact: fail closed, no recognized calibration.
  }
  return registry;
}
