// Reorder-safe avionics ingestion for the simulator viewer.
// Publication/receipt time is transport metadata: freshness advances only
// when a source group presents a new epoch/sequence.

import { firstFault } from "./wire-bounds.js";

export const COHERENCE = Object.freeze({
  INSUFFICIENT: "insufficient",
  COHERENT: "coherent",
  EXCESSIVE_SKEW: "excessive-skew",
});

export const INCARNATION_POLICY = Object.freeze({
  PIN_FIRST: "pin-first",
  SIM_ACCEPT_UNSEEN: "sim-accept-unseen",
});

const SERIAL_HALF_RANGE = 0x80000000;
const CLOCK_VEHICLE_BOOT = 1;
const CLOCK_SIMULATION = 2;
const CLOCK_HOST_MONOTONIC = 3;
// Source roles (LINK-04). Primary panels admit only the operational
// estimate; truth and FC state have their own consumers. Every consumer
// validates the COMPLETE stamp for its exact role.
export const ROLE = Object.freeze({
  OPERATIONAL_ESTIMATE: 1,
  SIMULATION_TRUTH: 2,
  FC_STATE: 3,
});
// Role-specific stamp rules: which clock domains a role may legitimately
// stamp. Estimates carry source clocks; truth is simulation-clocked; FC
// state is stamped at host receipt (its wire carries no source time).
const ROLE_CLOCKS = Object.freeze({
  [ROLE.OPERATIONAL_ESTIMATE]: [CLOCK_VEHICLE_BOOT, CLOCK_SIMULATION],
  [ROLE.SIMULATION_TRUTH]: [CLOCK_SIMULATION],
  [ROLE.FC_STATE]: [CLOCK_HOST_MONOTONIC],
});
const KNOWN_INTEGRITY = Object.freeze([1, 2, 3]);
const QUALITY_UNUSABLE = 2;
const ATTITUDE_VALID_FLAGS = 0b0011;
const KINEMATICS_VALID_FLAGS = 0b1100;
const KNOWN_VALID_FLAGS = ATTITUDE_VALID_FLAGS | KINEMATICS_VALID_FLAGS;

function increment(value, amount = 1) {
  return (value + amount) >>> 0;
}

function serialDistance(candidate, current) {
  return (candidate - current) >>> 0;
}

export function serialIsNewer(candidate, current) {
  const distance = serialDistance(candidate, current);
  return distance !== 0 && distance < SERIAL_HALF_RANGE;
}

// The first stamp field to violate its exact wire type, range, or role
// contract, as a typed `{ field, rule }` reason, or `null` when the stamp
// is valid for `role`: source id and acquisition time are u64 (BigInt,
// upper-bounded — a decoded varint past 2^64 is refused, not accepted),
// epoch and sequence are u32, the role must match the lane exactly, the
// clock must be legal for that role, and the integrity classification
// must be KNOWN (unspecified is a fault, not a default). Fail-closed,
// never clamped. This one validator serves the estimate, truth, and
// FC-state lanes so no lane ships weaker provenance checks.
export function stampFaultForRole(stamp, role) {
  if (stamp === null || stamp === undefined || typeof stamp !== "object") {
    return { field: "stamp", rule: "malformed" };
  }
  const fault = firstFault([
    ["sourceId", "u64", stamp.sourceId],
    ["sourceIncarnation", "incarnation", stamp.sourceIncarnation],
    ["sourceEpoch", "u32", stamp.sourceEpoch],
    ["sequence", "u32", stamp.sequence],
    ["acquiredAtNanos", "u64", stamp.acquiredAtNanos],
  ]);
  if (fault) return fault;
  if (stamp.role !== role) {
    return { field: "role", rule: "role-mismatch" };
  }
  if (!ROLE_CLOCKS[role].includes(stamp.clock)) {
    return { field: "clock", rule: "malformed" };
  }
  if (!KNOWN_INTEGRITY.includes(stamp.integrity)) {
    return { field: "integrity", rule: "unknown" };
  }
  return null;
}

function copyAttitude(avionics) {
  return Object.freeze({
    quat: Object.freeze({ ...avionics.quat }),
    rates: Object.freeze([...avionics.rates]),
    armState: avionics.armState >>> 0,
  });
}

function copyKinematics(avionics) {
  return Object.freeze({
    posNed: Object.freeze([...avionics.posNed]),
    velNed: Object.freeze([...avionics.velNed]),
    armState: avionics.armState >>> 0,
  });
}

function copyEstimatorStatus() {
  return Object.freeze({});
}

function stampCopy(stamp) {
  return Object.freeze({ ...stamp });
}

function stampsEqual(left, right) {
  return (
    left !== null &&
    left !== undefined &&
    right !== null &&
    right !== undefined &&
    left.sourceId === right.sourceId &&
    left.sourceIncarnation === right.sourceIncarnation &&
    left.sourceEpoch === right.sourceEpoch &&
    left.sequence === right.sequence &&
    left.acquiredAtNanos === right.acquiredAtNanos &&
    left.clock === right.clock
  );
}

// Whether `status` can vouch for a numeric group acquired at `numeric`:
// identical source identity and clock, and an acquisition gap within the
// coherence budget. The host merges lanes into one sample per tick, so a
// numeric group lawfully arrives alongside a status acquired a few
// milliseconds later (attitude, position, and status streams interleave
// at their own rates); demanding the exact same instant would strip a
// validly authorized lane on every interleaved arrival and flash the
// panels between valid and invalid. Beyond the budget — or across any
// identity or clock change — authorization still fails closed.
function acquisitionPaired(numeric, status, maximumSkewNanos) {
  if (
    numeric === null ||
    numeric === undefined ||
    status === null ||
    status === undefined ||
    numeric.sourceId !== status.sourceId ||
    numeric.sourceIncarnation !== status.sourceIncarnation ||
    numeric.sourceEpoch !== status.sourceEpoch ||
    numeric.clock !== status.clock
  ) {
    return false;
  }
  const skew =
    numeric.acquiredAtNanos >= status.acquiredAtNanos
      ? numeric.acquiredAtNanos - status.acquiredAtNanos
      : status.acquiredAtNanos - numeric.acquiredAtNanos;
  return skew <= maximumSkewNanos;
}

function groupSnapshot(group, nowMs) {
  if (!group) return null;
  return Object.freeze({
    ...group.data,
    stamp: group.stamp,
    ageMs: Math.max(0, nowMs - group.acceptedAtMs),
  });
}

function coherenceOf(attitude, kinematics, maximumSkewNanos) {
  if (!attitude || !kinematics) {
    return Object.freeze({ status: COHERENCE.INSUFFICIENT, skewNanos: null });
  }
  const a = attitude.stamp;
  const k = kinematics.stamp;
  if (
    a.sourceId !== k.sourceId ||
    a.sourceIncarnation !== k.sourceIncarnation ||
    a.sourceEpoch !== k.sourceEpoch ||
    a.clock !== k.clock
  ) {
    return Object.freeze({ status: COHERENCE.INSUFFICIENT, skewNanos: null });
  }
  const skew = a.acquiredAtNanos >= k.acquiredAtNanos
    ? a.acquiredAtNanos - k.acquiredAtNanos
    : k.acquiredAtNanos - a.acquiredAtNanos;
  return Object.freeze({
    status: skew <= maximumSkewNanos ? COHERENCE.COHERENT : COHERENCE.EXCESSIVE_SKEW,
    skewNanos: skew,
  });
}

export class AvionicsIngress {
  constructor({
    vehicleId,
    sourceId = null,
    sourceIncarnation = null,
    incarnationPolicy = INCARNATION_POLICY.PIN_FIRST,
    maximumSeenIncarnations = 8,
    maximumSkewNanos,
  }) {
    if (typeof vehicleId !== "bigint") throw new TypeError("vehicleId must be a bigint");
    if (sourceId !== null && typeof sourceId !== "bigint") {
      throw new TypeError("sourceId must be null or a bigint");
    }
    if (sourceIncarnation !== null && !/^[0-9a-f]{32}$/.test(sourceIncarnation)) {
      throw new TypeError("sourceIncarnation must be null or 32 lowercase hex characters");
    }
    if (!Object.values(INCARNATION_POLICY).includes(incarnationPolicy)) {
      throw new TypeError("unknown incarnationPolicy");
    }
    if (!Number.isInteger(maximumSeenIncarnations) || maximumSeenIncarnations < 1) {
      throw new TypeError("maximumSeenIncarnations must be a positive integer");
    }
    if (typeof maximumSkewNanos !== "bigint" || maximumSkewNanos < 0n) {
      throw new TypeError("maximumSkewNanos must be a non-negative bigint");
    }
    this.vehicleId = vehicleId;
    this.sourceId = sourceId;
    this.sourceIncarnation = sourceIncarnation;
    this.incarnationPolicy = incarnationPolicy;
    this.maximumSeenIncarnations = maximumSeenIncarnations;
    this.seenIncarnations = new Set();
    if (sourceIncarnation !== null) this.seenIncarnations.add(sourceIncarnation);
    this.maximumSkewNanos = maximumSkewNanos;
    this.sourceEpoch = null;
    this.attitude = null;
    this.kinematics = null;
    this.estimatorStatus = null;
    this.statusRegime = null;
    this.previousStatusRegime = null;
    this.attitudeAuthorizationPaired = false;
    this.kinematicsAuthorizationPaired = false;
    this.validFlags = 0;
    this.quality = QUALITY_UNUSABLE;
    this.generation = 0;
    this.lastCoherence = COHERENCE.INSUFFICIENT;
    this.lastRejectReason = null;
    this.counters = {
      duplicates: 0,
      reordered: 0,
      wrongVehicle: 0,
      wrongSource: 0,
      wrongIncarnation: 0,
      oldIncarnation: 0,
      incarnationTransitions: 0,
      incarnationCapacity: 0,
      oldEpoch: 0,
      sourceResets: 0,
      invalidStamps: 0,
      sequenceGaps: 0,
      excessiveSkew: 0,
      timeRegressions: 0,
      clockChanges: 0,
    };
  }

  ingest(message, nowMs) {
    if (!Number.isFinite(nowMs)) throw new TypeError("nowMs must be finite");
    if (message.vehicleId !== this.vehicleId) {
      this.bump("wrongVehicle");
      return false;
    }
    const avionics = message.avionics;
    if (!avionics) return false;

    const acceptedStatus = this.acceptGroup(
      "estimatorStatus",
      avionics.estimatorStatusStamp,
      copyEstimatorStatus,
      avionics,
      nowMs,
    );
    if (acceptedStatus) {
      // Each accepted status opens a new authorization regime; the one it
      // closes is retained so a numeric group acquired under the closed
      // regime (the host merges lanes into one sample per tick, so lanes
      // interleave with the status stream) is judged by the estimator
      // state that actually governed its acquisition instant.
      const s = avionics.estimatorStatusStamp;
      this.previousStatusRegime = this.statusRegime;
      this.statusRegime = Object.freeze({
        stamp: Object.freeze({
          sourceId: s.sourceId,
          sourceIncarnation: s.sourceIncarnation,
          sourceEpoch: s.sourceEpoch,
          acquiredAtNanos: s.acquiredAtNanos,
          clock: s.clock,
        }),
        validFlags: avionics.validFlags >>> 0,
        quality: avionics.quality >>> 0,
      });
    }
    const acceptedAttitude = this.acceptGroup(
      "attitude",
      avionics.attitudeStamp,
      copyAttitude,
      avionics,
      nowMs,
    );
    const acceptedKinematics = this.acceptGroup(
      "kinematics",
      avionics.kinematicsStamp,
      copyKinematics,
      avionics,
      nowMs,
    );
    const acceptedNumeric = acceptedAttitude || acceptedKinematics;
    const previousValidFlags = this.validFlags;
    const previousQuality = this.quality;
    this.updateAuthorization(avionics, acceptedAttitude, acceptedKinematics);
    const authorizationChanged =
      this.validFlags !== previousValidFlags || this.quality !== previousQuality;
    const changed = acceptedNumeric || acceptedStatus || authorizationChanged;
    if (changed) {
      this.generation = increment(this.generation);
      this.recordCoherenceTransition();
    }
    return changed;
  }

  updateAuthorization(avionics, acceptedAttitude, acceptedKinematics) {
    // A transport validation fault cannot mint a source acquisition time, so
    // a publication backed by the current stamp may change trust only in the
    // fail-closed direction and never refreshes that stamp's age.
    if (stampsEqual(avionics.estimatorStatusStamp, this.estimatorStatus?.stamp)) {
      this.applyStatusDowngrade(avionics);
    }
    if (acceptedAttitude || acceptedKinematics) {
      this.updateAuthorizationFromNumeric(avionics, acceptedAttitude, acceptedKinematics);
    }
  }

  applyStatusDowngrade(avionics) {
    if (!this.hasEstablishedAuthorization()) {
      this.failClosedAuthorization();
      return;
    }
    this.validFlags = (this.validFlags & (avionics.validFlags >>> 0)) >>> 0;
    this.quality = Math.max(this.quality, avionics.quality >>> 0);
  }

  // The regime whose declared estimator state governs a numeric group
  // acquired at `numericStamp`: the current status when acquired at or
  // after its instant, else the previous status when acquired within its
  // reign. The skew budget against the current status keeps a stale
  // numeric from borrowing authority across a stream gap; identity and
  // clock must match in every case, so nothing authorizes across a
  // source reset. Null means no status can vouch for this acquisition —
  // fail closed.
  authorizationRegimeFor(numericStamp) {
    const current = this.statusRegime;
    if (
      current === null ||
      !acquisitionPaired(numericStamp, current.stamp, this.maximumSkewNanos)
    ) {
      return null;
    }
    if (numericStamp.acquiredAtNanos >= current.stamp.acquiredAtNanos) return current;
    const previous = this.previousStatusRegime;
    if (
      previous !== null &&
      acquisitionPaired(numericStamp, previous.stamp, this.maximumSkewNanos) &&
      numericStamp.acquiredAtNanos >= previous.stamp.acquiredAtNanos
    ) {
      return previous;
    }
    return null;
  }

  updateAuthorizationFromNumeric(avionics, acceptedAttitude, acceptedKinematics) {
    const currentStatusStamp = this.estimatorStatus?.stamp;
    const statusMatches = stampsEqual(avionics.estimatorStatusStamp, currentStatusStamp);
    const incomingValidFlags = avionics.validFlags >>> 0;
    let pairedQuality = null;
    if (acceptedAttitude) {
      const regime = statusMatches
        ? this.authorizationRegimeFor(avionics.attitudeStamp)
        : null;
      this.attitudeAuthorizationPaired = regime !== null;
      const attitudeFlags =
        regime !== null ? regime.validFlags & incomingValidFlags & ATTITUDE_VALID_FLAGS : 0;
      this.validFlags = (
        (this.validFlags & ~ATTITUDE_VALID_FLAGS) | attitudeFlags
      ) >>> 0;
      if (regime !== null) {
        pairedQuality = Math.max(regime.quality, avionics.quality >>> 0);
      }
    }
    if (acceptedKinematics) {
      const regime = statusMatches
        ? this.authorizationRegimeFor(avionics.kinematicsStamp)
        : null;
      this.kinematicsAuthorizationPaired = regime !== null;
      const kinematicsFlags =
        regime !== null ? regime.validFlags & incomingValidFlags & KINEMATICS_VALID_FLAGS : 0;
      this.validFlags = (
        (this.validFlags & ~KINEMATICS_VALID_FLAGS) | kinematicsFlags
      ) >>> 0;
      if (regime !== null) {
        pairedQuality = Math.max(
          pairedQuality ?? 0,
          Math.max(regime.quality, avionics.quality >>> 0),
        );
      }
    }

    if ((this.validFlags & KNOWN_VALID_FLAGS) === 0) {
      this.quality = QUALITY_UNUSABLE;
      return;
    }
    if (pairedQuality !== null) this.quality = pairedQuality;
  }

  hasEstablishedAuthorization() {
    return (
      (this.attitude !== null && this.attitudeAuthorizationPaired) ||
      (this.kinematics !== null && this.kinematicsAuthorizationPaired)
    );
  }

  failClosedAuthorization() {
    this.attitudeAuthorizationPaired = false;
    this.kinematicsAuthorizationPaired = false;
    this.validFlags = 0;
    this.quality = QUALITY_UNUSABLE;
  }

  acceptGroup(name, stamp, copyData, avionics, nowMs) {
    if (stamp === null || stamp === undefined) return false;
    const fault = stampFaultForRole(stamp, ROLE.OPERATIONAL_ESTIMATE);
    if (fault !== null) {
      this.bump("invalidStamps");
      this.lastRejectReason = fault;
      return false;
    }
    if (!this.acceptSource(stamp.sourceId)) return false;
    if (!this.acceptIncarnation(stamp.sourceIncarnation)) return false;
    if (!this.acceptEpoch(stamp.sourceEpoch)) return false;

    const current = this[name];
    if (current && current.stamp.sequence === stamp.sequence) {
      this.bump("duplicates");
      return false;
    }
    if (current && !serialIsNewer(stamp.sequence, current.stamp.sequence)) {
      this.bump("reordered");
      return false;
    }
    if (current && stamp.clock !== current.stamp.clock) {
      this.bump("clockChanges");
      return false;
    }
    if (current && stamp.acquiredAtNanos <= current.stamp.acquiredAtNanos) {
      this.bump("timeRegressions");
      return false;
    }
    if (current) {
      const gap = serialDistance(stamp.sequence, current.stamp.sequence);
      if (gap > 1) this.bump("sequenceGaps", gap - 1);
    }
    this[name] = Object.freeze({
      stamp: stampCopy(stamp),
      data: copyData(avionics),
      acceptedAtMs: nowMs,
    });
    return true;
  }

  acceptSource(candidate) {
    if (this.sourceId === null) {
      this.sourceId = candidate;
      return true;
    }
    if (candidate === this.sourceId) return true;
    this.bump("wrongSource");
    return false;
  }

  acceptIncarnation(candidate) {
    if (this.sourceIncarnation === null) {
      this.sourceIncarnation = candidate;
      this.seenIncarnations.add(candidate);
      return true;
    }
    if (candidate === this.sourceIncarnation) return true;
    if (this.seenIncarnations.has(candidate)) {
      this.bump("oldIncarnation");
      return false;
    }
    if (this.incarnationPolicy !== INCARNATION_POLICY.SIM_ACCEPT_UNSEEN) {
      this.bump("wrongIncarnation");
      return false;
    }
    if (this.seenIncarnations.size >= this.maximumSeenIncarnations) {
      this.bump("incarnationCapacity");
      return false;
    }
    this.seenIncarnations.add(candidate);
    this.sourceIncarnation = candidate;
    this.sourceEpoch = null;
    this.attitude = null;
    this.kinematics = null;
    this.estimatorStatus = null;
    this.statusRegime = null;
    this.previousStatusRegime = null;
    this.failClosedAuthorization();
    this.lastCoherence = COHERENCE.INSUFFICIENT;
    this.bump("incarnationTransitions");
    return true;
  }

  acceptEpoch(candidate) {
    if (this.sourceEpoch === null) {
      this.sourceEpoch = candidate;
      return true;
    }
    if (candidate === this.sourceEpoch) return true;
    if (!serialIsNewer(candidate, this.sourceEpoch)) {
      this.bump("oldEpoch");
      return false;
    }
    this.sourceEpoch = candidate;
    this.attitude = null;
    this.kinematics = null;
    this.estimatorStatus = null;
    this.statusRegime = null;
    this.previousStatusRegime = null;
    this.failClosedAuthorization();
    this.bump("sourceResets");
    return true;
  }

  recordCoherenceTransition() {
    const coherence = coherenceOf(this.attitude, this.kinematics, this.maximumSkewNanos);
    if (
      coherence.status === COHERENCE.EXCESSIVE_SKEW &&
      this.lastCoherence !== COHERENCE.EXCESSIVE_SKEW
    ) {
      this.bump("excessiveSkew");
    }
    this.lastCoherence = coherence.status;
  }

  snapshot(nowMs) {
    if (!Number.isFinite(nowMs)) throw new TypeError("nowMs must be finite");
    const attitude = groupSnapshot(this.attitude, nowMs);
    const kinematics = groupSnapshot(this.kinematics, nowMs);
    const estimatorStatus = groupSnapshot(this.estimatorStatus, nowMs);
    return Object.freeze({
      generation: this.generation,
      sourceId: this.sourceId,
      sourceIncarnation: this.sourceIncarnation,
      sourceEpoch: this.sourceEpoch,
      attitude,
      kinematics,
      estimatorStatus,
      validFlags: this.validFlags,
      quality: this.quality,
      coherence: coherenceOf(this.attitude, this.kinematics, this.maximumSkewNanos),
    });
  }

  diagnostics() {
    return Object.freeze({ ...this.counters, lastRejectReason: this.lastRejectReason });
  }

  bump(name, amount = 1) {
    this.counters[name] = increment(this.counters[name], amount);
  }
}

// FC-state freshness, fail closed. A report is accepted only when its
// COMPLETE stamp validates for the FC-state role; the source identity
// (id + incarnation) is pinned at first acceptance for the session; the
// epoch/sequence pair must strictly ADVANCE in wrapping serial order —
// duplicates and reordered/older reports never refresh age and never
// regress the displayed state; and the arm value itself must be in
// range. Heartbeat loss surfaces as stale instead of a forever-fresh
// arm state.
export class FcStateTracker {
  constructor(staleAfterMs = 3000) {
    this.staleAfterMs = staleAfterMs;
    this.last = null;
  }

  // Feeds one decoded fcState lane (or null) and returns the current
  // view. Only a NEW report — pinned identity, epoch advanced, or same
  // epoch with the sequence strictly newer in wrapping order — restarts
  // the age clock.
  observe(fcState, nowMs) {
    if (this.accepts(fcState)) {
      const stamp = fcState.stamp;
      this.last = {
        armState: fcState.armState >>> 0,
        sourceId: stamp.sourceId,
        sourceIncarnation: stamp.sourceIncarnation,
        sourceEpoch: stamp.sourceEpoch,
        sequence: stamp.sequence,
        firstSeenMs: nowMs,
      };
    }
    return this.view(nowMs);
  }

  // Whether a report is a valid, strictly-new observation from the
  // pinned source. Every rejection is fail-closed: the previous view
  // (and its age) stands.
  accepts(fcState) {
    if (!fcState) return false;
    if (stampFaultForRole(fcState.stamp, ROLE.FC_STATE) !== null) return false;
    const armState = fcState.armState;
    if (!Number.isInteger(armState) || armState < 0 || armState > 2) return false;
    const last = this.last;
    if (last === null) return true;
    const stamp = fcState.stamp;
    // Identity is pinned for the session: a different source id or
    // incarnation is not this FC's report stream.
    if (stamp.sourceId !== last.sourceId || stamp.sourceIncarnation !== last.sourceIncarnation) {
      return false;
    }
    if (stamp.sourceEpoch === last.sourceEpoch) {
      return serialIsNewer(stamp.sequence, last.sequence);
    }
    // A newer epoch (FC restart/re-attach) restarts the numbering; an
    // older epoch is a replay.
    return serialIsNewer(stamp.sourceEpoch, last.sourceEpoch);
  }

  // The display view: null before any report; stale once the newest
  // report's age exceeds the threshold.
  view(nowMs) {
    if (this.last === null) return null;
    const ageMs = nowMs - this.last.firstSeenMs;
    return {
      armState: this.last.armState,
      ageMs,
      stale: ageMs > this.staleAfterMs,
    };
  }
}
