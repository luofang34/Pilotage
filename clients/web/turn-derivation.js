// Measurement-coherent turn-rate derivation for the simulator feeder
// (DYN-01). Heading rate is differenced ONLY between two distinct
// accepted heading measurements of one stream — identified by the
// AV-01 stamp (sourceId + sourceIncarnation + sourceEpoch, ordered by
// sequence) — over the MEASUREMENT acquisition clock, never browser
// render or receipt time. Repeated renders of one sample re-declare
// the cached rate (the measurement's own age carries staleness);
// duplicates and reordered samples never advance the state; a stream
// discontinuity or session retirement resets it, so no difference can
// ever straddle two sessions, sources, or epochs.

import { serialIsNewer } from "./telemetry-ingress.js";

// Bounds on the measurement-clock interval between the two differenced
// samples: closer than the minimum is too noisy to differentiate,
// farther than the maximum is stale for a rate. Both yield no sample —
// never a wild or frozen rate.
export const MIN_TURN_DT_MS = 5;
export const MAX_TURN_DT_MS = 500;

const TURN_BASIS_HEADING_RATE = 0;

function sameStream(a, b) {
  return (
    a.sourceId === b.sourceId &&
    a.sourceIncarnation === b.sourceIncarnation &&
    a.sourceEpoch === b.sourceEpoch &&
    a.clock === b.clock
  );
}

/** Derives heading-rate dynamics declarations from per-measurement
 * heading samples. One instance per session presentation. */
export class TurnDerivation {
  #prev;
  #cached;

  constructor() {
    this.reset();
  }

  /** Clears all state; the next sample can never difference against
   * anything observed before the reset. */
  reset() {
    this.#prev = null;
    this.#cached = null;
  }

  /**
   * Consumes the current declared heading (radians) with its
   * measurement stamp; returns a dynamics declaration for writeState
   * or null when no rate can honestly be derived.
   *
   * - No heading/stamp: full reset (a gap must not bridge).
   * - Stream change (source/incarnation/epoch/clock): reset, then seed.
   * - Same sequence: repeated render — re-declare the cached rate with
   *   the measurement's current age; no state advance.
   * - Serially older sequence: reordered — ignored entirely.
   * - Newer sequence: difference over acquiredAtNanos within the
   *   documented bounds; out-of-bound dt seeds but declares nothing.
   */
  update(headingRad, ageMs, stamp) {
    if (!Number.isFinite(headingRad) || !stamp) {
      this.reset();
      return null;
    }
    const prev = this.#prev;
    if (prev === null || !sameStream(prev.stamp, stamp)) {
      this.reset();
      this.#prev = { headingRad, stamp };
      return null;
    }
    if (stamp.sequence === prev.stamp.sequence) {
      return this.#declare(ageMs);
    }
    if (!serialIsNewer(stamp.sequence, prev.stamp.sequence)) {
      return this.#declare(ageMs);
    }
    const dtMs = Number(stamp.acquiredAtNanos - prev.stamp.acquiredAtNanos) / 1e6;
    this.#prev = { headingRad, stamp };
    if (!(dtMs >= MIN_TURN_DT_MS && dtMs <= MAX_TURN_DT_MS)) {
      this.#cached = null;
      return null;
    }
    // Circular difference into (-π, π] so 359°→1° is +2°, never −358°.
    let delta = (headingRad - prev.headingRad) % (2 * Math.PI);
    if (delta > Math.PI) delta -= 2 * Math.PI;
    if (delta <= -Math.PI) delta += 2 * Math.PI;
    this.#cached = { turnBasis: TURN_BASIS_HEADING_RATE, turnRps: delta / (dtMs / 1000) };
    return this.#declare(ageMs);
  }

  #declare(ageMs) {
    if (this.#cached === null) return null;
    return { ...this.#cached, ageMs };
  }
}
