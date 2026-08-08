// Reorder-safe avionics ingestion — thin wrappers over the shared
// feeder core (#252). Publication/receipt time is transport metadata:
// freshness advances only when a source group presents a new
// epoch/sequence. Every semantic judgement (identity pinning,
// incarnation policy, wrap-safe ordering, coherence, the fail-closed
// authorization regimes) runs in indicate-instrument-feeder via the
// instrument wasm build; this module keeps decode-shape validation of
// the script's own object dialect, marshalling, and the constructor
// contracts its consumers rely on.

import { firstFault } from "./wire-bounds.js";
import { bindings, deepFreeze } from "./feeder-wasm.js";

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
// Source roles (LINK-04). Primary panels admit only the operational
// estimate; truth and FC state have their own consumers. Every consumer
// validates the COMPLETE stamp for its exact role.
export const ROLE = Object.freeze({
  OPERATIONAL_ESTIMATE: 1,
  SIMULATION_TRUTH: 2,
  FC_STATE: 3,
  NAVIGATION_SOLUTION: 6,
});

function serialDistance(candidate, current) {
  return (candidate - current) >>> 0;
}

function byteCode(value) {
  return Number.isInteger(value) && value >= 0 && value <= 255;
}

export function serialIsNewer(candidate, current) {
  const distance = serialDistance(candidate, current);
  return distance !== 0 && distance < SERIAL_HALF_RANGE;
}

// The first stamp field to violate its exact wire type, range, or role
// contract, as a typed `{ field, rule }` reason, or `null` when the
// stamp is valid for `role`. Shape rules (u64/u32 bounds, the
// incarnation encoding) belong to this decode boundary; the role, clock
// and integrity legality verdict comes from the feeder core so no lane
// ships weaker provenance checks than the shells that link it directly.
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
  // Role, clock, and integrity are byte codings on the wire; a value
  // outside 0..=255 must be rejected here, never truncated into range
  // by the wasm boundary's u8 parameters (a 257 would otherwise pass
  // validation mod 256 and then fault mid-marshal).
  if (!byteCode(stamp.clock)) return { field: "clock", rule: "malformed" };
  if (!byteCode(stamp.integrity)) return { field: "integrity", rule: "malformed" };
  if (Number.isInteger(stamp.role) && !byteCode(stamp.role)) {
    return { field: "role", rule: "malformed" };
  }
  // With the wasm unavailable there is no role/clock/integrity verdict;
  // fail closed rather than pass a stamp half-validated.
  if (bindings === null) return { field: "stamp", rule: "malformed" };
  const verdict = bindings.feeder_stamp_fault(
    Number.isInteger(stamp.role) ? stamp.role : 0,
    stamp.clock,
    stamp.integrity,
    role,
  );
  if (verdict === null || verdict === undefined) return null;
  const [field, rule] = verdict.split(":");
  return { field, rule };
}

function rawStamp(stamp) {
  return {
    role: stamp.role,
    integrity: stamp.integrity,
    sourceId: stamp.sourceId,
    sourceIncarnation: stamp.sourceIncarnation,
    sourceEpoch: stamp.sourceEpoch,
    sequence: stamp.sequence,
    acquiredAtNanos: stamp.acquiredAtNanos,
    clock: stamp.clock,
  };
}

export class AvionicsIngress {
  #inner;
  #vehicleId;
  #invalidStamps;
  #lastRejectReason;
  #generationPatch;
  #counterPatches;
  #countersProxy;

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
    this.#vehicleId = vehicleId;
    this.#invalidStamps = 0;
    this.#lastRejectReason = null;
    this.#generationPatch = null;
    this.#counterPatches = new Map();
    this.#countersProxy = null;
    // With the wasm unavailable the ingress degrades to admitting
    // nothing — the panels report unavailable through the instruments
    // module's own fail-visible path.
    this.#inner =
      bindings === null
        ? null
        : new bindings.FeederIngress(
            vehicleId,
            sourceId === null ? undefined : sourceId,
            sourceIncarnation ?? "",
            incarnationPolicy === INCARNATION_POLICY.SIM_ACCEPT_UNSEEN,
            maximumSeenIncarnations,
            maximumSkewNanos,
          );
  }

  ingest(message, nowMs) {
    if (!Number.isFinite(nowMs)) throw new TypeError("nowMs must be finite");
    if (this.#inner === null) return false;
    const avionics = message.avionics;
    if (message.vehicleId === this.#vehicleId && !avionics) return false;

    // Decode-shape validation happens at this boundary; a faulty stamp
    // is stripped so the core never sees it, and the refusal is counted
    // here with the script's `{field, rule}` vocabulary. The core
    // refuses a wrong-vehicle publication before reading its stamps;
    // matching that order keeps invalidStamps meaning "this vehicle's
    // stamps were bad".
    const foreign = message.vehicleId !== this.#vehicleId;
    const admit = (stamp) => {
      if (foreign || stamp === null || stamp === undefined) return null;
      const fault = stampFaultForRole(stamp, ROLE.OPERATIONAL_ESTIMATE);
      if (fault !== null) {
        this.#invalidStamps = (this.#invalidStamps + 1) >>> 0;
        this.#lastRejectReason = fault;
        return null;
      }
      return rawStamp(stamp);
    };
    // The status stamp is special: a re-published status whose only
    // fault is a corrupt role/integrity byte must still fold the
    // fail-closed duplicate-status downgrade (its six identity fields
    // are what the regime matches on). When the identity survives shape
    // validation, the stamp passes through with the legality verdict
    // poisoned so the core refuses it in its own ladder (which counts
    // the invalid stamp) while the downgrade still applies; identity
    // damage falls back to stripping, exactly like the other lanes.
    const admitStatus = (stamp) => {
      if (foreign || stamp === null || stamp === undefined) return null;
      const fault = stampFaultForRole(stamp, ROLE.OPERATIONAL_ESTIMATE);
      if (fault === null) return rawStamp(stamp);
      this.#lastRejectReason = fault;
      const identityIntact =
        firstFault([
          ["sourceId", "u64", stamp.sourceId],
          ["sourceIncarnation", "incarnation", stamp.sourceIncarnation],
          ["sourceEpoch", "u32", stamp.sourceEpoch],
          ["sequence", "u32", stamp.sequence],
          ["acquiredAtNanos", "u64", stamp.acquiredAtNanos],
        ]) === null && byteCode(stamp.clock);
      if (!identityIntact) {
        this.#invalidStamps = (this.#invalidStamps + 1) >>> 0;
        return null;
      }
      return { ...rawStamp(stamp), role: 0, integrity: 0 };
    };
    const sample = {
      vehicleId: message.vehicleId,
      quat: {
        w: avionics?.quat?.w ?? 0,
        x: avionics?.quat?.x ?? 0,
        y: avionics?.quat?.y ?? 0,
        z: avionics?.quat?.z ?? 0,
      },
      rates: [avionics?.rates?.[0] ?? 0, avionics?.rates?.[1] ?? 0, avionics?.rates?.[2] ?? 0],
      posNed: [avionics?.posNed?.[0] ?? 0, avionics?.posNed?.[1] ?? 0, avionics?.posNed?.[2] ?? 0],
      velNed: [avionics?.velNed?.[0] ?? 0, avionics?.velNed?.[1] ?? 0, avionics?.velNed?.[2] ?? 0],
      armState: avionics?.armState >>> 0,
      validFlags: avionics?.validFlags >>> 0,
      quality: avionics?.quality >>> 0,
      attitudeStamp: admit(avionics?.attitudeStamp),
      kinematicsStamp: admit(avionics?.kinematicsStamp),
      estimatorStatusStamp: admitStatus(avionics?.estimatorStatusStamp),
    };
    return this.#inner.ingest(sample, nowMs);
  }

  snapshot(nowMs) {
    if (!Number.isFinite(nowMs)) throw new TypeError("nowMs must be finite");
    if (this.#inner === null) {
      return deepFreeze({
        generation: 0,
        sourceId: null,
        sourceIncarnation: null,
        sourceEpoch: null,
        attitude: null,
        kinematics: null,
        estimatorStatus: null,
        validFlags: 0,
        quality: 2,
        coherence: { status: COHERENCE.INSUFFICIENT, skewNanos: null },
      });
    }
    const snapshot = this.#inner.snapshot(nowMs);
    if (this.#generationPatch !== null) {
      const { assigned, base } = this.#generationPatch;
      snapshot.generation = (assigned + (snapshot.generation - base)) >>> 0;
    }
    return deepFreeze(snapshot);
  }

  // The generation and counter fields remain assignable, as they were
  // when this class held them directly: an assignment rebases the
  // published value, and subsequent counting continues from it under
  // the same wrapping arithmetic.
  get generation() {
    return this.snapshot(0).generation;
  }

  set generation(value) {
    if (this.#inner === null) return;
    this.#generationPatch = {
      assigned: value >>> 0,
      base: this.#inner.snapshot(0).generation,
    };
  }

  get counters() {
    this.#countersProxy ??= new Proxy(
      {},
      {
        get: (_target, field) => this.diagnostics()[field],
        set: (_target, field, value) => {
          this.#counterPatches.set(field, {
            assigned: value >>> 0,
            base: this.#rawCounters()[field] >>> 0,
          });
          return true;
        },
        // Membership, enumeration, and descriptors resolve one
        // own-property model, as a plain object would: the counter
        // fields plus any patched-in assignment are enumerable own
        // properties; lastRejectReason is an own property held out of
        // enumeration (it was never a counter); nothing else exists.
        has: (_target, field) =>
          field === "lastRejectReason" || this.#counterFields().has(field),
        ownKeys: () => {
          const keys = this.#counterFields();
          keys.add("lastRejectReason");
          return [...keys];
        },
        getOwnPropertyDescriptor: (_target, field) => {
          if (field !== "lastRejectReason" && !this.#counterFields().has(field)) {
            return undefined;
          }
          return {
            value: this.diagnostics()[field],
            writable: true,
            enumerable: field !== "lastRejectReason",
            configurable: true,
          };
        },
      },
    );
    return this.#countersProxy;
  }

  #counterFields() {
    const keys = new Set(Object.keys(this.#rawCounters()));
    for (const field of this.#counterPatches.keys()) keys.add(field);
    return keys;
  }

  #rawCounters() {
    if (this.#inner === null) {
      return {
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
        invalidStamps: this.#invalidStamps,
        sequenceGaps: 0,
        excessiveSkew: 0,
        timeRegressions: 0,
        clockChanges: 0,
      };
    }
    const counters = this.#inner.diagnostics();
    counters.invalidStamps = (counters.invalidStamps + this.#invalidStamps) >>> 0;
    return counters;
  }

  diagnostics() {
    const counters = this.#rawCounters();
    for (const [field, { assigned, base }] of this.#counterPatches) {
      counters[field] = (assigned + ((counters[field] >>> 0) - base)) >>> 0;
    }
    return Object.freeze({
      ...counters,
      lastRejectReason: this.#lastRejectReason,
    });
  }
}

// FC-state freshness, fail closed. A report is accepted only when its
// COMPLETE stamp validates for the FC-state role; the source identity
// is pinned at first acceptance for the session; the epoch/sequence
// pair must strictly ADVANCE in wrapping serial order; and the arm
// value itself must be in range. Heartbeat loss surfaces as stale
// instead of a forever-fresh arm state.
export class FcStateTracker {
  #inner;

  constructor(staleAfterMs = 3000) {
    this.#inner = bindings === null ? null : new bindings.FeederFcState(staleAfterMs);
  }

  // Feeds one decoded fcState lane (or null) and returns the current
  // view. Only a NEW report restarts the age clock.
  observe(fcState, nowMs) {
    if (this.#inner === null) return null;
    return this.#inner.observe(this.#marshal(fcState), nowMs);
  }

  // The display view: null before any report; stale once the newest
  // report's age exceeds the threshold.
  view(nowMs) {
    if (this.#inner === null) return null;
    return this.#inner.observe(null, nowMs);
  }

  #marshal(fcState) {
    if (!fcState) return null;
    if (stampFaultForRole(fcState.stamp, ROLE.FC_STATE) !== null) return null;
    const armState = fcState.armState;
    if (!Number.isInteger(armState) || armState < 0) return null;
    return {
      stamp: rawStamp(fcState.stamp),
      armState,
      lastCommand: sanitizeFcCommand(fcState.lastCommand),
    };
  }
}

// The FC's arm/disarm COMMAND_ACK verdict riding the fc-state lane. A
// malformed verdict degrades to null (no verdict) rather than rejecting
// the whole report: the arm state itself is still valid and fresh.
function sanitizeFcCommand(lastCommand) {
  if (!lastCommand || typeof lastCommand.arm !== "boolean") return null;
  const result = lastCommand.result;
  if (!Number.isInteger(result) || result < 0) return null;
  return { arm: lastCommand.arm, result };
}

// Navigation guidance freshness, fail closed (ADR-0031). The sample's
// own values stay raw here — meters and radians — and reach the
// instrument's dot scale only through the display profile.
export class NavGuidanceTracker {
  #inner;

  constructor() {
    this.#inner = bindings === null ? null : new bindings.FeederNavGuidance();
  }

  // Feeds one decoded navGuidance lane (or null) and returns the
  // current snapshot. Only a NEW sample restarts the age clock.
  observe(navGuidance, nowMs) {
    if (!Number.isFinite(nowMs)) throw new TypeError("nowMs must be finite");
    if (this.#inner === null) return null;
    return deepFreeze(this.#inner.observe(this.#marshal(navGuidance), nowMs));
  }

  // The display view: null before any accepted sample.
  snapshot(nowMs) {
    if (!Number.isFinite(nowMs)) throw new TypeError("nowMs must be finite");
    if (this.#inner === null) return null;
    return deepFreeze(this.#inner.snapshot(nowMs));
  }

  diagnostics() {
    if (this.#inner === null) {
      return Object.freeze({
        accepted: 0,
        invalidStamps: 0,
        wrongSource: 0,
        duplicates: 0,
        malformedGuidance: 0,
        lastRejectReason: null,
      });
    }
    return Object.freeze(this.#inner.diagnostics());
  }

  #marshal(navGuidance) {
    if (!navGuidance) return null;
    // A shape-faulted stamp is poisoned (role 0) so the core counts an
    // invalid stamp with the same vocabulary the script always used.
    const stampOk = stampFaultForRole(navGuidance.stamp, ROLE.NAVIGATION_SOLUTION) === null;
    const stamp = stampOk
      ? rawStamp(navGuidance.stamp)
      : {
          role: 0,
          integrity: 0,
          sourceId: 0n,
          sourceIncarnation: "00000000000000000000000000000000",
          sourceEpoch: 0,
          sequence: 0,
          acquiredAtNanos: 0n,
          clock: 0,
        };
    // Uninterpretable guidance values poison the course, which the core
    // refuses as malformed guidance — never a partial display.
    const counted = [navGuidance.legIndex, navGuidance.waypointCount, navGuidance.solutionQuality];
    const wellShaped =
      counted.every((value) => Number.isInteger(value) && value >= 0) &&
      typeof navGuidance.toIdent === "string" &&
      typeof navGuidance.fromIdent === "string" &&
      [
        navGuidance.courseRad,
        navGuidance.lateralDeviationM,
        navGuidance.verticalDeviationM,
        navGuidance.distanceToWaypointM,
      ].every((value) => typeof value === "number");
    return {
      stamp,
      toIdent: wellShaped ? navGuidance.toIdent : "",
      fromIdent: wellShaped ? navGuidance.fromIdent : "",
      courseRad: wellShaped ? navGuidance.courseRad : NaN,
      lateralDeviationM: wellShaped ? navGuidance.lateralDeviationM : NaN,
      verticalDeviationM: wellShaped ? navGuidance.verticalDeviationM : NaN,
      distanceToWaypointM: wellShaped ? navGuidance.distanceToWaypointM : NaN,
      legIndex: wellShaped ? navGuidance.legIndex : 0,
      waypointCount: wellShaped ? navGuidance.waypointCount : 0,
      solutionQuality: wellShaped ? navGuidance.solutionQuality : 0,
    };
  }
}
