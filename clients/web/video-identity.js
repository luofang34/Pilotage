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
import { RULE, firstFault, isIncarnation, isU32 } from "./wire-bounds.js";

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
  // The gate is the final authority: any malformed wire field (out-of-range,
  // wrong numeric kind) can never produce a ready verdict, even when the mapping
  // and calibration are otherwise valid. Validate before reading mappingAvailable
  // or any mapped/BigInt field, so a wrong-numeric-kind value fails closed with a
  // typed reason rather than throwing. This also closes the direct associate()
  // path, which does not run admit()'s validator.
  const fault = metaFault(meta);
  if (fault !== null) return closed(`malformed-meta:${fault.field}:${fault.rule}`);
  if (meta.mappingAvailable !== true) return closed("mapping-unavailable");
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

const KNOWN_CLOCKS = new Set([CLOCK_VEHICLE_BOOT, CLOCK_SIMULATION]);

/**
 * The first frame-metadata field to violate its exact wire type and range, as a
 * typed `{ field, rule }` reason, or `null` when every field is valid. Covers
 * EVERY wire field the video path reads, stores, or maps — not only the identity
 * tuple but the clock-mapping offset (a signed i64) and the error-bound, receive,
 * and publication times (u64). Callers must run this before any BigInt
 * arithmetic on those fields, so a wrong-numeric-kind value fails closed with a
 * reason instead of throwing a `TypeError` when mixed with a BigInt. Negative,
 * fractional, over-range, or wrong-numeric-kind values are refused, never
 * clamped.
 */
export function metaFault(meta) {
  if (meta === null || typeof meta !== "object") return { field: "meta", rule: RULE.MALFORMED };
  const fault = firstFault([
    ["sourceId", "u8", meta.sourceId],
    ["sourceEpoch", "u32", meta.sourceEpoch],
    ["sequence", "u32", meta.sequence],
    ["cameraId", "u32", meta.cameraId],
    ["calibrationId", "u32", meta.calibrationId],
    ["captureTimeNanos", "u64", meta.captureTimeNanos],
    ["mappingOffsetNanos", "i64", meta.mappingOffsetNanos],
    ["clockErrorBoundNanos", "u64", meta.clockErrorBoundNanos],
    ["receiveTimeNanos", "u64", meta.receiveTimeNanos],
    ["publicationTimeNanos", "u64", meta.publicationTimeNanos],
  ]);
  if (fault !== null) return fault;
  if (!isIncarnation(meta.sourceIncarnation)) return { field: "sourceIncarnation", rule: RULE.MALFORMED };
  if (!KNOWN_CLOCKS.has(meta.captureClock)) return { field: "captureClock", rule: RULE.MALFORMED };
  if (!KNOWN_CLOCKS.has(meta.mappingTargetClock)) return { field: "mappingTargetClock", rule: RULE.MALFORMED };
  return null;
}

function result(reason, discontinuity, fault = null) {
  return Object.freeze({
    accepted: reason === ADMIT.ACCEPTED,
    reason,
    discontinuity,
    fault,
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
    // The `{ field, rule }` of the most recent malformed frame, so diagnostics
    // can report which wire field failed and why — not merely that a frame was
    // malformed.
    this.lastMalformedReason = null;
  }

  admit(meta) {
    const fault = metaFault(meta);
    if (fault !== null) {
      this.bump("malformed");
      this.lastMalformedReason = fault;
      return result(ADMIT.MALFORMED, false, fault);
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
    return Object.freeze({ ...this.counters, lastMalformedReason: this.lastMalformedReason });
  }

  bump(name, amount = 1) {
    this.counters[name] = (this.counters[name] + amount) >>> 0;
  }
}
