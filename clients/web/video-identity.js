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

const CLOCK_VEHICLE_BOOT = 1;
const CLOCK_SIMULATION = 2;
const INCARNATION_HEX = /^[0-9a-f]{32}$/;

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
 * Verdict on whether a frame's capture clock can anchor a conformal overlay.
 *
 * Defaults to unavailable: only an explicitly available, bounded clock mapping
 * yields `conformalReady === true`, and it carries the quantified error bound a
 * consumer would budget against. Conformal drawing itself does not exist yet;
 * this states only admissibility, so an absent or unavailable mapping keeps the
 * gate closed.
 */
export function conformalGate(meta) {
  if (!meta || meta.mappingAvailable !== true) {
    return Object.freeze({
      mappingValid: false,
      conformalReady: false,
      clockErrorBoundNanos: null,
    });
  }
  return Object.freeze({
    mappingValid: true,
    conformalReady: true,
    clockErrorBoundNanos: meta.clockErrorBoundNanos,
  });
}

function isMetaValid(meta) {
  return (
    meta !== null &&
    typeof meta === "object" &&
    Number.isInteger(meta.sourceId) &&
    typeof meta.sourceIncarnation === "string" &&
    INCARNATION_HEX.test(meta.sourceIncarnation) &&
    Number.isInteger(meta.sourceEpoch) &&
    meta.sourceEpoch >= 0 &&
    meta.sourceEpoch <= 0xffff_ffff &&
    Number.isInteger(meta.sequence) &&
    meta.sequence >= 0 &&
    meta.sequence <= 0xffff_ffff &&
    typeof meta.captureTimeNanos === "bigint" &&
    meta.captureTimeNanos >= 0n &&
    Number.isInteger(meta.cameraId) &&
    (meta.captureClock === CLOCK_VEHICLE_BOOT || meta.captureClock === CLOCK_SIMULATION)
  );
}

function result(reason, discontinuity, meta) {
  return Object.freeze({
    accepted: reason === ADMIT.ACCEPTED,
    reason,
    discontinuity,
    gate: conformalGate(meta),
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
    };
  }

  admit(meta) {
    if (!isMetaValid(meta)) {
      this.bump("malformed");
      return result(ADMIT.MALFORMED, false, meta);
    }
    const state = this.sources.get(meta.sourceId);
    if (!state) {
      this.establish(meta);
      this.bump("accepted");
      return result(ADMIT.ACCEPTED, true, meta);
    }
    if (meta.cameraId !== state.cameraId) {
      this.bump("wrongCamera");
      return result(ADMIT.WRONG_CAMERA, false, meta);
    }
    if (meta.sourceIncarnation !== state.incarnation) {
      this.establish(meta);
      this.bump("incarnationResets");
      this.bump("accepted");
      return result(ADMIT.ACCEPTED, true, meta);
    }
    return this.admitWithinIncarnation(state, meta);
  }

  admitWithinIncarnation(state, meta) {
    if (meta.sourceEpoch !== state.epoch) {
      if (!serialIsNewer(meta.sourceEpoch, state.epoch)) {
        this.bump("staleEpochs");
        return result(ADMIT.STALE_EPOCH, false, meta);
      }
      this.establish(meta);
      this.bump("epochResets");
      this.bump("accepted");
      return result(ADMIT.ACCEPTED, true, meta);
    }
    if (meta.sequence === state.lastSequence) {
      this.bump("duplicates");
      return result(ADMIT.DUPLICATE, false, meta);
    }
    if (!serialIsNewer(meta.sequence, state.lastSequence)) {
      this.bump("reordered");
      return result(ADMIT.REORDERED, false, meta);
    }
    this.advance(state, meta);
    this.bump("accepted");
    return result(ADMIT.ACCEPTED, false, meta);
  }

  establish(meta) {
    this.sources.set(meta.sourceId, {
      cameraId: meta.cameraId,
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
