// Capture-to-aircraft-snapshot association for the viewer (ADR-0020).
//
// Given a v2 video frame with a valid clock mapping, this finds the aircraft
// snapshot that corresponds to the frame's CAPTURE time — never browser receipt
// time — with a traceable identity and a quantified total error. It OBSERVES
// the aircraft snapshots main.js has already accepted (it does not gate them;
// AV-01's ingestion contract in telemetry-ingress.js owns that) and keeps a
// bounded history ring. Association maps the capture time through the frame's
// CaptureClockMapping into the snapshot clock domain and selects the nearest
// snapshot by acquisition time.
//
// The current aircraft stream is identified by the full AV-01 tuple
// (sourceId + sourceIncarnation + sourceEpoch). Only snapshots from that stream
// are eligible: if the nearest snapshot by time belongs to a superseded source,
// incarnation, or epoch, the association is an explicit discontinuity, never a
// ready verdict — a snapshot from before a source switch or an epoch reset can
// never anchor a conformal overlay even when it is closest in time.
//
// Everything fails closed: an empty history, a clock-domain mismatch, a
// stream-identity discontinuity, or a total error (mapping error bound +
// association delta) over budget all yield "not ready". The result is finally
// passed through conformalGate, so its checks (mapping validity, target-clock
// match, published/recognized calibration, overflow-safe mapped time, error
// budget) can never be bypassed.

import { conformalGate, mapCaptureTime, DEFAULT_MAX_CLOCK_ERROR_NANOS } from "./video-identity.js";
import { firstFault } from "./wire-bounds.js";

const CLOCK_VEHICLE_BOOT = 1;
const CLOCK_SIMULATION = 2;

// History ring size. The mapped capture time of a displayed frame is at most a
// glass-to-glass latency old (transport + host queue + decode, well under a
// second in this demo), so a few seconds of snapshots is ample slack. At the
// simulator's telemetry rate (tens of hertz) 256 entries covers several
// seconds; overflow drops the oldest (counted), never silently. A power of two
// keeps the intent obvious rather than implying a tuned value.
export const DEFAULT_HISTORY_CAPACITY = 256;

/** Reason an association was or was not produced, for logging and diagnostics. */
export const ASSOCIATION = Object.freeze({
  READY: "ready",
  MAPPING_UNAVAILABLE: "mapping-unavailable",
  MAPPED_TIME_OVERFLOW: "mapped-time-overflow",
  EMPTY_HISTORY: "empty-history",
  CLOCK_MISMATCH: "clock-mismatch",
  STREAM_DISCONTINUITY: "stream-discontinuity",
  TOTAL_ERROR_EXCEEDS_BUDGET: "total-error-exceeds-budget",
  NOT_ADMITTED: "not-admitted",
});

// The first field of an AV-01 snapshot identity to violate its exact wire type
// and range, as a typed `{ field, rule }` reason, or `null` when it is valid:
// source id is a u64 (a BigInt, never a truncating Number), epoch and sequence
// are u32, and the acquisition time is a u64. Out-of-range, negative,
// fractional, or wrong-numeric-kind values are refused fail-closed, never
// clamped.
function snapshotIdentityFault(id) {
  if (id === null || typeof id !== "object") return { field: "identity", rule: "malformed" };
  const fault = firstFault([
    ["sourceId", "u64", id.sourceId],
    ["sourceIncarnation", "incarnation", id.sourceIncarnation],
    ["sourceEpoch", "u32", id.sourceEpoch],
    ["sequence", "u32", id.sequence],
    ["acquiredAtNanos", "u64", id.acquiredAtNanos],
  ]);
  if (fault) return fault;
  if (id.clock !== CLOCK_VEHICLE_BOOT && id.clock !== CLOCK_SIMULATION) {
    return { field: "clock", rule: "malformed" };
  }
  return null;
}

function identityOf(id) {
  return Object.freeze({
    sourceId: id.sourceId,
    sourceIncarnation: id.sourceIncarnation,
    sourceEpoch: id.sourceEpoch,
    sequence: id.sequence,
    acquiredAtNanos: id.acquiredAtNanos,
    clock: id.clock,
  });
}

// The stream identity: the AV-01 tuple that must be continuous for a snapshot
// to anchor the same conformal timeline. A change in any of sourceId (a
// different source), sourceIncarnation (a source restart), or sourceEpoch (an
// epoch reset) begins a new stream.
function streamOf(id) {
  return Object.freeze({
    sourceId: id.sourceId,
    sourceIncarnation: id.sourceIncarnation,
    sourceEpoch: id.sourceEpoch,
  });
}

function sameStream(id, stream) {
  return (
    stream !== null &&
    id.sourceId === stream.sourceId &&
    id.sourceIncarnation === stream.sourceIncarnation &&
    id.sourceEpoch === stream.sourceEpoch
  );
}

function sameIdentity(a, b) {
  return (
    a.sourceId === b.sourceId &&
    a.sourceIncarnation === b.sourceIncarnation &&
    a.sourceEpoch === b.sourceEpoch &&
    a.sequence === b.sequence &&
    a.acquiredAtNanos === b.acquiredAtNanos &&
    a.clock === b.clock
  );
}

function absDiff(a, b) {
  return a >= b ? a - b : b - a;
}

function closed(reason) {
  return Object.freeze({
    ready: false,
    snapshotIdentity: null,
    mappedCaptureNanos: null,
    totalErrorNanos: null,
    reason,
  });
}

/**
 * Bounded history of accepted aircraft snapshots and the association logic over
 * it. `observe` records one accepted snapshot's AV-01 identity; `associate`
 * finds the snapshot corresponding to a frame's capture time.
 */
export class SnapshotAssociator {
  constructor({ capacity = DEFAULT_HISTORY_CAPACITY } = {}) {
    if (!Number.isInteger(capacity) || capacity < 1) {
      throw new TypeError("capacity must be a positive integer");
    }
    this.capacity = capacity;
    this.entries = [];
    this.currentStream = null;
    this.counters = { observed: 0, deduped: 0, dropped: 0, invalid: 0 };
    this.lastInvalidReason = null;
  }

  /** Records one accepted snapshot identity, oldest-first, and adopts its
   *  stream as the current one. Consecutive duplicates are ignored (main.js
   *  re-reads the accepted snapshot each telemetry frame); overflow drops the
   *  oldest entry and is counted. */
  observe(identity) {
    const fault = snapshotIdentityFault(identity);
    if (fault !== null) {
      this.bump("invalid");
      this.lastInvalidReason = fault;
      return false;
    }
    const entry = identityOf(identity);
    const newest = this.entries[this.entries.length - 1];
    if (newest && sameIdentity(newest, entry)) {
      this.bump("deduped");
      return false;
    }
    this.entries.push(entry);
    this.currentStream = streamOf(entry);
    this.bump("observed");
    if (this.entries.length > this.capacity) {
      this.entries.shift();
      this.bump("dropped");
    }
    return true;
  }

  /** Clears the history and current stream. main.js calls this on a
   *  transport/session reset so a new session can never associate a frame
   *  against a snapshot the previous session left in the ring. */
  reset() {
    this.entries = [];
    this.currentStream = null;
  }

  /**
   * Associates a frame's capture time with the nearest accepted snapshot.
   * Returns a verdict `{ ready, snapshotIdentity, mappedCaptureNanos,
   * totalErrorNanos, reason }`. Fails closed; the `reason` is one of
   * [`ASSOCIATION`]. The budget boundary is inclusive: a total error exactly
   * equal to the budget is within budget.
   */
  associate(meta, options = {}) {
    const budget = options.maxClockErrorNanos ?? DEFAULT_MAX_CLOCK_ERROR_NANOS;
    if (!meta || meta.mappingAvailable !== true) return closed(ASSOCIATION.MAPPING_UNAVAILABLE);
    const mapped = mapCaptureTime(meta.captureTimeNanos, meta.mappingOffsetNanos);
    if (mapped === null) return closed(ASSOCIATION.MAPPED_TIME_OVERFLOW);
    if (this.entries.length === 0) return closed(ASSOCIATION.EMPTY_HISTORY);
    const candidates = this.entries.filter((e) => e.clock === meta.mappingTargetClock);
    if (candidates.length === 0) return closed(ASSOCIATION.CLOCK_MISMATCH);

    let nearest = null;
    let bestDelta = null;
    for (const entry of candidates) {
      const delta = absDiff(entry.acquiredAtNanos, mapped);
      if (bestDelta === null || delta < bestDelta) {
        bestDelta = delta;
        nearest = entry;
      }
    }
    // The nearest snapshot by time must belong to the current stream. If it is
    // from a superseded source, incarnation, or epoch, the aircraft identity
    // changed between that snapshot and now, so no ready association is
    // possible — even though it is closest in time.
    if (!sameStream(nearest, this.currentStream)) {
      return closed(ASSOCIATION.STREAM_DISCONTINUITY);
    }

    const totalError = meta.clockErrorBoundNanos + bestDelta;
    const snapshotIdentity = identityOf(nearest);
    if (totalError > budget) {
      return this.verdict(false, snapshotIdentity, mapped, totalError, ASSOCIATION.TOTAL_ERROR_EXCEEDS_BUDGET);
    }
    // conformalGate is the final authority: it re-checks mapping validity, the
    // target-clock match against the associated snapshot, the calibration, and
    // the mapping's own error bound. Association never bypasses it.
    const gate = conformalGate(meta, snapshotIdentity, options);
    if (!gate.conformalReady) {
      return this.verdict(false, snapshotIdentity, mapped, totalError, `gate-closed:${gate.reason}`);
    }
    return this.verdict(true, snapshotIdentity, mapped, totalError, ASSOCIATION.READY);
  }

  verdict(ready, snapshotIdentity, mappedCaptureNanos, totalErrorNanos, reason) {
    return Object.freeze({ ready, snapshotIdentity, mappedCaptureNanos, totalErrorNanos, reason });
  }

  diagnostics() {
    return Object.freeze({ ...this.counters, size: this.entries.length, lastInvalidReason: this.lastInvalidReason });
  }

  bump(name, amount = 1) {
    this.counters[name] = (this.counters[name] + amount) >>> 0;
  }
}

/**
 * The single admission-then-association path. Association runs ONLY on a frame
 * the identity tracker accepted, so a duplicate, reordered, or stale-epoch frame
 * (which the tracker rejects) can never produce a fresh association. Returns
 * `{ accepted, admit, association }`; `association` is `null` when the frame was
 * not admitted, else the associator's verdict.
 */
export function associateIfAccepted(tracker, associator, meta, options = {}) {
  const admit = tracker.admit(meta);
  if (!admit.accepted) {
    return Object.freeze({ accepted: false, admit, association: closed(ASSOCIATION.NOT_ADMITTED) });
  }
  return Object.freeze({ accepted: true, admit, association: associator.associate(meta, options) });
}
