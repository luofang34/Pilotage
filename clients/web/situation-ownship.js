// The vehicle's own position on the situation map.
//
// The mark is drawn from a geodetic fix and from nothing else. Absence of a
// fix is absence of the mark, with the reason on the stage: a map that drew
// a vehicle at a default, at a last-known position, or at a place derived
// from a local origin would be telling a reader where the vehicle is when
// it does not know (ADR-0022, ADR-0037).

/** Why the map is drawing no vehicle. */
export const OWNSHIP_REASON = Object.freeze({
  NO_SAMPLE: "OWNSHIP_NO_TELEMETRY",
  NO_FIX: "OWNSHIP_NO_FIX",
});

/** How long a fix may go unrefreshed before the mark is withdrawn. A
 *  position that stopped arriving is a position the vehicle has left. */
export const OWNSHIP_STALE_AFTER_MS = 3_000;

/**
 * The vehicle position a telemetry sample states, or a typed reason it
 * states none.
 *
 * The fix comes from the simulation-truth lane. Which lane a group belongs
 * to is settled in the decoder, which refuses a truth group stamped with
 * another role outright, so a mislabelled group arrives here as no group at
 * all. This side does not repeat that gate: it would name a reason the
 * decoder makes unreachable, and a reason the client cannot produce is a
 * reason that lies when a reader meets it.
 */
export function ownshipFromTelemetry(telemetry) {
  const truth = telemetry?.simTruth;
  if (!truth) return { position: null, reason: OWNSHIP_REASON.NO_SAMPLE };
  const fix = truth.geodetic;
  if (!fix) return { position: null, reason: OWNSHIP_REASON.NO_FIX };
  return {
    position: {
      latitudeDeg: fix.latitudeDeg,
      longitudeDeg: fix.longitudeDeg,
      heightM: fix.heightM,
    },
    reason: null,
  };
}

/**
 * Wires the mark to a map. Returns `observe` for each telemetry sample and
 * `age`, which withdraws a mark whose fix stopped arriving.
 *
 * Both take the reader's own monotonic clock, never the sample's simulation
 * clock: the rule measures how long ago a fix reached this page, which is
 * the question a stale mark raises. Staleness cannot be decided inside
 * `observe` alone, because a link that goes silent delivers no sample to
 * decide it with, and the mark would sit at its last position for as long
 * as the page stayed open.
 */
export function attachOwnship(maplibre, map, surface) {
  const element = surface.ownerDocument.createElement("div");
  element.className = "map-ownship";
  element.setAttribute("role", "img");
  const marker = new maplibre.Marker({ element });
  let shown = false;
  let lastFixAt = null;
  let label = null;

  const withdraw = (reason) => {
    if (shown) {
      marker.remove();
      shown = false;
    }
    lastFixAt = null;
    surface.dataset.ownship = "absent";
    surface.dataset.ownshipReason = reason;
    // A position beside an absent mark is a position a reader can still
    // read off the surface.
    delete surface.dataset.ownshipPosition;
  };

  const age = (nowMs) => {
    if (lastFixAt === null) return;
    if (nowMs - lastFixAt <= OWNSHIP_STALE_AFTER_MS) return;
    withdraw(OWNSHIP_REASON.NO_FIX);
  };

  const observe = (telemetry, nowMs) => {
    const { position, reason } = ownshipFromTelemetry(telemetry);
    if (position === null) {
      // One sample without a fix does not blink the mark; the age rule is
      // what removes it, and it applies whether or not samples keep coming.
      if (lastFixAt !== null) {
        age(nowMs);
        return;
      }
      withdraw(reason);
      return;
    }
    lastFixAt = nowMs;
    marker.setLngLat([position.longitudeDeg, position.latitudeDeg]);
    if (!shown) {
      marker.addTo(map);
      shown = true;
    }
    const next = `Vehicle at ${position.latitudeDeg.toFixed(5)}, ${position.longitudeDeg.toFixed(5)}`;
    // An accessible name rewritten at the telemetry rate is announced at
    // the telemetry rate.
    if (next !== label) {
      element.setAttribute("aria-label", next);
      label = next;
    }
    surface.dataset.ownship = "shown";
    surface.dataset.ownshipPosition =
      `${position.latitudeDeg.toFixed(6)},${position.longitudeDeg.toFixed(6)}`;
    delete surface.dataset.ownshipReason;
  };

  withdraw(OWNSHIP_REASON.NO_SAMPLE);
  return { observe, age, marker };
}
