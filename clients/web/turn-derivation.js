// Measurement-coherent turn-rate derivation (DYN-01) — a thin wrapper
// over the shared feeder core (#252). The semantics (stream identity,
// wrap-safe ordering, dt bounds, circular differencing) run in
// pilotage-instrument-feeder via the instrument wasm build; this module
// keeps only the script-side stamp dialect (string clock names) and
// marshalling.

import { bindings } from "./feeder-wasm.js";

// Bounds on the measurement-clock interval between the two differenced
// samples, mirrored from the feeder crate: closer than the minimum is
// too noisy to differentiate, farther than the maximum is stale for a
// rate. Both yield no sample — never a wild or frozen rate.
export const MIN_TURN_DT_MS = 5;
export const MAX_TURN_DT_MS = 500;

// Stamps arrive in two dialects: wire-decoded stamps already carry the
// numeric u8 clock code and pass through untouched; script-built stamps
// name their clock and map through this table. An unknown name maps to
// 0, which the core treats as its own stream key.
const CLOCK_CODES = Object.freeze({
  "vehicle-boot": 1,
  simulation: 2,
  "host-monotonic": 3,
});

function rawStamp(stamp) {
  return {
    // Role and integrity do not participate in stream identity for the
    // derivation; fixed legal codes keep the marshalled stamp complete.
    role: 1,
    integrity: 2,
    sourceId: stamp.sourceId,
    sourceIncarnation: stamp.sourceIncarnation,
    sourceEpoch: stamp.sourceEpoch >>> 0,
    sequence: stamp.sequence >>> 0,
    acquiredAtNanos: stamp.acquiredAtNanos,
    clock: typeof stamp.clock === "number" ? stamp.clock : (CLOCK_CODES[stamp.clock] ?? 0),
  };
}

/** Derives heading-rate dynamics declarations from per-measurement
 * heading samples. One instance per session presentation. */
export class TurnDerivation {
  #inner;

  constructor() {
    // With the wasm unavailable the derivation degrades to declaring
    // nothing — fail-closed, never a substitute rate.
    this.#inner = bindings === null ? null : new bindings.FeederTurn();
  }

  /** Clears all state; the next sample can never difference against
   * anything observed before the reset. */
  reset() {
    this.#inner?.reset();
  }

  /**
   * Consumes the current declared heading (radians) with its
   * measurement stamp; returns a dynamics declaration for writeState
   * or null when no rate can honestly be derived.
   */
  update(headingRad, ageMs, stamp) {
    if (this.#inner === null) return null;
    const marshalled =
      stamp && typeof stamp === "object" && typeof stamp.acquiredAtNanos === "bigint"
        ? rawStamp(stamp)
        : null;
    return this.#inner.update(headingRad, ageMs, marshalled);
  }
}
