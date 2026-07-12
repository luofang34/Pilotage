// Reorder-safe avionics ingestion for the simulator viewer.
// Publication/receipt time is transport metadata: freshness advances only
// when a source group presents a new epoch/sequence.

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

function isStampValid(stamp) {
  return (
    stamp !== null &&
    typeof stamp === "object" &&
    typeof stamp.sourceId === "bigint" &&
    typeof stamp.sourceIncarnation === "string" &&
    /^[0-9a-f]{32}$/.test(stamp.sourceIncarnation) &&
    Number.isInteger(stamp.sourceEpoch) &&
    stamp.sourceEpoch >= 0 &&
    stamp.sourceEpoch <= 0xffff_ffff &&
    Number.isInteger(stamp.sequence) &&
    stamp.sequence >= 0 &&
    stamp.sequence <= 0xffff_ffff &&
    typeof stamp.acquiredAtNanos === "bigint" &&
    stamp.acquiredAtNanos >= 0n &&
    (stamp.clock === CLOCK_VEHICLE_BOOT || stamp.clock === CLOCK_SIMULATION)
  );
}

function copyAttitude(avionics) {
  return Object.freeze({
    quat: Object.freeze({ ...avionics.quat }),
    rates: Object.freeze([...avionics.rates]),
    validFlags: avionics.validFlags >>> 0,
    quality: avionics.quality >>> 0,
    armState: avionics.armState >>> 0,
  });
}

function copyKinematics(avionics) {
  return Object.freeze({
    posNed: Object.freeze([...avionics.posNed]),
    velNed: Object.freeze([...avionics.velNed]),
    validFlags: avionics.validFlags >>> 0,
    quality: avionics.quality >>> 0,
    armState: avionics.armState >>> 0,
  });
}

function stampCopy(stamp) {
  return Object.freeze({ ...stamp });
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
    this.generation = 0;
    this.lastCoherence = COHERENCE.INSUFFICIENT;
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

    let changed = false;
    changed = this.acceptGroup("attitude", avionics.attitudeStamp, copyAttitude, avionics, nowMs)
      || changed;
    changed = this.acceptGroup(
      "kinematics",
      avionics.kinematicsStamp,
      copyKinematics,
      avionics,
      nowMs,
    ) || changed;
    if (changed) {
      this.generation = increment(this.generation);
      this.recordCoherenceTransition();
    }
    return changed;
  }

  acceptGroup(name, stamp, copyData, avionics, nowMs) {
    if (stamp === null) return false;
    if (!isStampValid(stamp)) {
      this.bump("invalidStamps");
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
    return Object.freeze({
      generation: this.generation,
      sourceId: this.sourceId,
      sourceIncarnation: this.sourceIncarnation,
      sourceEpoch: this.sourceEpoch,
      attitude,
      kinematics,
      coherence: coherenceOf(this.attitude, this.kinematics, this.maximumSkewNanos),
    });
  }

  diagnostics() {
    return Object.freeze({ ...this.counters });
  }

  bump(name, amount = 1) {
    this.counters[name] = increment(this.counters[name], amount);
  }
}
