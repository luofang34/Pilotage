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

// The first stamp field to violate its exact wire type and range, as a typed
// `{ field, rule }` reason, or `null` when the stamp is valid: source id and
// acquisition time are u64 (BigInt, upper-bounded — a decoded varint past 2^64
// is refused, not accepted), epoch and sequence are u32. Fail-closed, never
// clamped; this tightens identity validation without changing the
// reorder/freshness gating below.
function stampFault(stamp) {
  if (stamp === null || typeof stamp !== "object") return { field: "stamp", rule: "malformed" };
  const fault = firstFault([
    ["sourceId", "u64", stamp.sourceId],
    ["sourceIncarnation", "incarnation", stamp.sourceIncarnation],
    ["sourceEpoch", "u32", stamp.sourceEpoch],
    ["sequence", "u32", stamp.sequence],
    ["acquiredAtNanos", "u64", stamp.acquiredAtNanos],
  ]);
  if (fault) return fault;
  if (stamp.clock !== CLOCK_VEHICLE_BOOT && stamp.clock !== CLOCK_SIMULATION) {
    return { field: "clock", rule: "malformed" };
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

function sameAcquisition(left, right) {
  return (
    left !== null &&
    left !== undefined &&
    right !== null &&
    right !== undefined &&
    left.sourceId === right.sourceId &&
    left.sourceIncarnation === right.sourceIncarnation &&
    left.sourceEpoch === right.sourceEpoch &&
    left.acquiredAtNanos === right.acquiredAtNanos &&
    left.clock === right.clock
  );
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

  updateAuthorizationFromNumeric(avionics, acceptedAttitude, acceptedKinematics) {
    const currentStatusStamp = this.estimatorStatus?.stamp;
    const statusMatches = stampsEqual(avionics.estimatorStatusStamp, currentStatusStamp);
    const incomingValidFlags = avionics.validFlags >>> 0;
    let acceptedExactPair = false;
    if (acceptedAttitude) {
      this.attitudeAuthorizationPaired =
        statusMatches && sameAcquisition(avionics.attitudeStamp, currentStatusStamp);
      const attitudeFlags = this.attitudeAuthorizationPaired
        ? incomingValidFlags & ATTITUDE_VALID_FLAGS
        : 0;
      this.validFlags = (
        (this.validFlags & ~ATTITUDE_VALID_FLAGS) | attitudeFlags
      ) >>> 0;
      acceptedExactPair = this.attitudeAuthorizationPaired;
    }
    if (acceptedKinematics) {
      this.kinematicsAuthorizationPaired =
        statusMatches && sameAcquisition(avionics.kinematicsStamp, currentStatusStamp);
      const kinematicsFlags = this.kinematicsAuthorizationPaired
        ? incomingValidFlags & KINEMATICS_VALID_FLAGS
        : 0;
      this.validFlags = (
        (this.validFlags & ~KINEMATICS_VALID_FLAGS) | kinematicsFlags
      ) >>> 0;
      acceptedExactPair = acceptedExactPair || this.kinematicsAuthorizationPaired;
    }

    if ((this.validFlags & KNOWN_VALID_FLAGS) === 0) {
      this.quality = QUALITY_UNUSABLE;
      return;
    }
    if (acceptedExactPair) this.quality = avionics.quality >>> 0;
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
    const fault = stampFault(stamp);
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
