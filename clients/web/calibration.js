// Published simulator camera calibration: load, hash-verify, and expose the
// calibration ids recognized for a given frame (ADR-0021).
//
// SIM / NOT FOR FLIGHT. The artifact describes a SIMULATED pinhole camera and a
// synthetic design eye; it is never real HUD optics and must not be read as
// optical qualification. The published artifact carries the exact canonical
// bytes the recorded SHA-256 was taken over (mirroring
// pilotage_adapter_api::calibration::canonical); this module recomputes the
// hash over those bytes and fails closed on any mismatch, then parses only the
// header fields it needs to gate conformal output. Everything here feeds the
// HUD-01 conformal gate's recognizedCalibrations: a missing, mismatched,
// corrupt, expired, or wrong-camera calibration yields no recognized id, so the
// gate stays closed.

// Mirrors CALIBRATION_SCHEMA_VERSION in the Rust canonical module.
export const CALIBRATION_SCHEMA_VERSION = 1;
// ValidityStatus::Valid.
const STATUS_VALID = 1;
// Byte length of the canonical header (schema .. to_frame); a body shorter than
// this cannot carry the fields the gate needs.
const CALIBRATION_HEADER_LEN = 45;

// Canonical header offsets (little-endian), mirroring the Rust `write_header`.
const OFFSET = Object.freeze({
  schema: 0,
  calibrationId: 2,
  cameraId: 6,
  version: 10,
  effectiveStart: 20,
  effectiveEnd: 28,
  status: 40,
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

/**
 * Loads and hash-verifies one published calibration artifact
 * `{ canonicalBase64, recordedHashHex }`. Returns `{ ok: true, ... }` with the
 * parsed header fields, or `{ ok: false, reason }` for a missing, corrupt,
 * hash-mismatched, or wrong-schema artifact. Fails closed throughout.
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
  if (bytes.length < CALIBRATION_HEADER_LEN) return { ok: false, reason: "corrupt" };
  const computed = await sha256Hex(bytes);
  if (computed !== artifact.recordedHashHex) return { ok: false, reason: "hash-mismatch" };
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
  if (view.getUint16(OFFSET.schema, true) !== CALIBRATION_SCHEMA_VERSION) {
    return { ok: false, reason: "schema-mismatch" };
  }
  return {
    ok: true,
    calibrationId: view.getUint32(OFFSET.calibrationId, true),
    cameraId: view.getUint32(OFFSET.cameraId, true),
    version: view.getUint32(OFFSET.version, true),
    effectiveStartNs: view.getBigUint64(OFFSET.effectiveStart, true),
    effectiveEndNs: view.getBigUint64(OFFSET.effectiveEnd, true),
    status: bytes[OFFSET.status],
  };
}

/**
 * A set of hash-verified calibrations. `recognizedFor` returns the calibration
 * ids admissible for a frame from `cameraId` at `nowUnixNs` (a bigint of Unix
 * nanoseconds): status Valid, within the effective window, and matching the
 * camera. A calibration failing any of those contributes nothing, so the
 * conformal gate stays closed.
 */
export class CalibrationRegistry {
  constructor() {
    this.calibrations = [];
  }

  /** Adds a loaded calibration; a failed load (`ok: false`) is ignored. */
  add(loaded) {
    if (loaded && loaded.ok === true) this.calibrations.push(loaded);
    return this;
  }

  recognizedFor(cameraId, nowUnixNs) {
    const recognized = new Set();
    for (const cal of this.calibrations) {
      if (cal.status !== STATUS_VALID) continue;
      if (!(nowUnixNs >= cal.effectiveStartNs && nowUnixNs < cal.effectiveEndNs)) continue;
      if (cal.cameraId !== cameraId) continue;
      recognized.add(cal.calibrationId);
    }
    return recognized;
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
