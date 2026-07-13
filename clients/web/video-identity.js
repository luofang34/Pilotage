// Per-source video capture identity tracking for the viewer (ADR-0020).
//
// A displayed frame must correspond to the aircraft state at capture, so a
// stale, duplicated, or reordered frame must never masquerade as fresh. This
// mirrors the avionics ingestion discipline in telemetry-ingress.js: freshness
// advances only when a source presents a strictly newer epoch/sequence, and a
// replayed frame leaves the accepted state — and therefore the displayed age —
// untouched. The serial-number comparison is reused from that module rather
// than reimplemented, so both paths share one definition of "newer".

import { serialIsNewer } from "./telemetry-ingress.js";
import { isIncarnation, isU32, isU64, isU8 } from "./wire-bounds.js";

const CLOCK_VEHICLE_BOOT = 1;
const CLOCK_SIMULATION = 2;
const U64_MAX = (1n << 64n) - 1n;

// Maximum clock-mapping error a conformal overlay tolerates. A bounded mapping
// alone is not sufficient: at a representative approach speed (~80 m/s) a 1 ms
// clock error is ~8 cm of along-track registration error, so the clock's
// contribution is budgeted to a small fraction of that. This is a SIM default;
// an airborne profile must derive its own budget from the intended display
// function and the platform's actual clock discipline.
export const DEFAULT_MAX_CLOCK_ERROR_NANOS = 2_000_000n;

// Calibration identities this consumer recognizes. Empty by default and
// fail-closed: a conformal overlay needs a known calibration, and the simulator
// publishes none (CalibrationId::NONE == 0), so the gate stays closed until a
// real calibration registry is supplied.
export const DEFAULT_RECOGNIZED_CALIBRATIONS = Object.freeze(new Set());

/** Applies a mapping offset to a capture time, refusing (returns `null`) when
 *  the signed offset would carry the result outside the u64 nanosecond range,
 *  rather than wrapping into a plausible-looking but wrong time. */
export function mapCaptureTime(captureTimeNanos, offsetNanos) {
  if (typeof captureTimeNanos !== "bigint" || typeof offsetNanos !== "bigint") return null;
  const mapped = captureTimeNanos + offsetNanos;
  if (mapped < 0n || mapped > U64_MAX) return null;
  return mapped;
}

/** Why a frame was or was not admitted, for logging and diagnostics. */
export const ADMIT = Object.freeze({
  ACCEPTED: "accepted",
  DUPLICATE: "duplicate",
  REORDERED: "reordered",
  STALE_EPOCH: "stale-epoch",
  WRONG_CAMERA: "wrong-camera",
  MALFORMED: "malformed",
});

/**
 * Verdict on whether a frame may anchor a conformal overlay against a specific
 * candidate aircraft snapshot. Consumes BOTH the frame metadata and the
 * candidate snapshot identity (an AV-01 `MeasurementStamp`, of which only the
 * `clock` is read here); a mapping is only usable if it targets the same clock
 * the snapshot is expressed in.
 *
 * Fails closed. `mappingValid` requires the mapping to be available, to target
 * the candidate snapshot's clock, to be within the configured error budget, and
 * to map the capture time without overflow. `conformalReady` additionally
 * requires a published, recognized calibration. A bounded mapping alone is not
 * sufficient, and conformal drawing itself does not exist yet; this states only
 * admissibility.
 *
 * `options.recognizedCalibrations` (a `Set` of calibration ids) and
 * `options.maxClockErrorNanos` (a `bigint`) override the fail-closed defaults.
 */
export function conformalGate(meta, snapshotIdentity, options = {}) {
  const recognized = options.recognizedCalibrations ?? DEFAULT_RECOGNIZED_CALIBRATIONS;
  const budget = options.maxClockErrorNanos ?? DEFAULT_MAX_CLOCK_ERROR_NANOS;
  const closed = (reason) =>
    Object.freeze({
      mappingValid: false,
      conformalReady: false,
      clockErrorBoundNanos: null,
      mappedCaptureTimeNanos: null,
      reason,
    });
  if (!meta || meta.mappingAvailable !== true) return closed("mapping-unavailable");
  // The gate is the final authority: malformed identity fields (out-of-range,
  // wrong numeric kind) can never produce a ready verdict, even when the
  // mapping and calibration are otherwise valid. This also closes the direct
  // associate() path, which does not run admit()'s validator.
  if (!isMetaValid(meta)) return closed("malformed-meta");
  if (!snapshotIdentity || snapshotIdentity.clock !== meta.mappingTargetClock) {
    return closed("clock-mismatch");
  }
  const bound = meta.clockErrorBoundNanos;
  if (typeof bound !== "bigint" || bound < 0n || bound > budget) {
    return closed("error-exceeds-budget");
  }
  const mapped = mapCaptureTime(meta.captureTimeNanos, meta.mappingOffsetNanos);
  if (mapped === null) return closed("mapped-time-overflow");
  // The clock side is valid: available, targets the snapshot's clock, within
  // budget, and maps without overflow. Conformal readiness additionally demands
  // a published, recognized calibration.
  const calibrationReady =
    isU32(meta.calibrationId) &&
    meta.calibrationId !== 0 &&
    recognized.has(meta.calibrationId);
  return Object.freeze({
    mappingValid: true,
    conformalReady: calibrationReady,
    clockErrorBoundNanos: bound,
    mappedCaptureTimeNanos: mapped,
    reason: calibrationReady ? "ready" : "calibration-unavailable",
  });
}

function isMetaValid(meta) {
  // Each field at its exact wire type and range: the video-frame source id is
  // a u8, epoch/sequence/camera id/calibration id are u32, and the capture
  // time is a u64 (a BigInt, never a truncating Number). Negative, fractional,
  // over-range, or wrong-numeric-kind values are refused fail-closed.
  return (
    meta !== null &&
    typeof meta === "object" &&
    isU8(meta.sourceId) &&
    isIncarnation(meta.sourceIncarnation) &&
    isU32(meta.sourceEpoch) &&
    isU32(meta.sequence) &&
    isU64(meta.captureTimeNanos) &&
    isU32(meta.cameraId) &&
    isU32(meta.calibrationId) &&
    (meta.captureClock === CLOCK_VEHICLE_BOOT || meta.captureClock === CLOCK_SIMULATION)
  );
}

function result(reason, discontinuity) {
  return Object.freeze({
    accepted: reason === ADMIT.ACCEPTED,
    reason,
    discontinuity,
  });
}

/**
 * Tracks capture identity independently per routing source. `admit` returns
 * whether a frame advances that source's timeline; only an advancing frame
 * updates the accepted state (last sequence, capture time, receive time), so a
 * rejected replay cannot refresh the frame's age.
 */
export class VideoIdentityTracker {
  constructor() {
    this.sources = new Map();
    this.counters = {
      accepted: 0,
      duplicates: 0,
      reordered: 0,
      staleEpochs: 0,
      wrongCamera: 0,
      malformed: 0,
      epochResets: 0,
      incarnationResets: 0,
      calibrationResets: 0,
    };
  }

  admit(meta) {
    if (!isMetaValid(meta)) {
      this.bump("malformed");
      return result(ADMIT.MALFORMED, false);
    }
    const state = this.sources.get(meta.sourceId);
    if (!state) {
      this.establish(meta);
      this.bump("accepted");
      return result(ADMIT.ACCEPTED, true);
    }
    if (meta.cameraId !== state.cameraId) {
      this.bump("wrongCamera");
      return result(ADMIT.WRONG_CAMERA, false);
    }
    if (meta.sourceIncarnation !== state.incarnation) {
      this.establish(meta);
      this.bump("incarnationResets");
      this.bump("accepted");
      return result(ADMIT.ACCEPTED, true);
    }
    // A calibration change re-bases the camera model, so the conformal timeline
    // must not silently continue across it: re-establish and flag a
    // discontinuity, exactly as for a fresh incarnation.
    if (meta.calibrationId !== state.calibrationId) {
      this.establish(meta);
      this.bump("calibrationResets");
      this.bump("accepted");
      return result(ADMIT.ACCEPTED, true);
    }
    return this.admitWithinIncarnation(state, meta);
  }

  admitWithinIncarnation(state, meta) {
    if (meta.sourceEpoch !== state.epoch) {
      if (!serialIsNewer(meta.sourceEpoch, state.epoch)) {
        this.bump("staleEpochs");
        return result(ADMIT.STALE_EPOCH, false);
      }
      this.establish(meta);
      this.bump("epochResets");
      this.bump("accepted");
      return result(ADMIT.ACCEPTED, true);
    }
    if (meta.sequence === state.lastSequence) {
      this.bump("duplicates");
      return result(ADMIT.DUPLICATE, false);
    }
    if (!serialIsNewer(meta.sequence, state.lastSequence)) {
      this.bump("reordered");
      return result(ADMIT.REORDERED, false);
    }
    this.advance(state, meta);
    this.bump("accepted");
    return result(ADMIT.ACCEPTED, false);
  }

  establish(meta) {
    this.sources.set(meta.sourceId, {
      cameraId: meta.cameraId,
      calibrationId: meta.calibrationId,
      incarnation: meta.sourceIncarnation,
      epoch: meta.sourceEpoch,
      lastSequence: meta.sequence,
      lastCaptureTimeNanos: meta.captureTimeNanos,
      lastReceiveTimeNanos: meta.receiveTimeNanos,
    });
  }

  advance(state, meta) {
    state.lastSequence = meta.sequence;
    state.lastCaptureTimeNanos = meta.captureTimeNanos;
    state.lastReceiveTimeNanos = meta.receiveTimeNanos;
  }

  /** The last accepted frame's identity for a source, or `null` if none yet. */
  lastAccepted(sourceId) {
    const state = this.sources.get(sourceId);
    if (!state) return null;
    return Object.freeze({
      cameraId: state.cameraId,
      calibrationId: state.calibrationId,
      incarnation: state.incarnation,
      epoch: state.epoch,
      sequence: state.lastSequence,
      captureTimeNanos: state.lastCaptureTimeNanos,
      receiveTimeNanos: state.lastReceiveTimeNanos,
    });
  }

  diagnostics() {
    return Object.freeze({ ...this.counters });
  }

  bump(name, amount = 1) {
    this.counters[name] = (this.counters[name] + amount) >>> 0;
  }
}
