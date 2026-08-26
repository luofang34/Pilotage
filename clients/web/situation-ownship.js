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
  STOPPED: "OWNSHIP_FIX_STOPPED",
});

/** Which measurement the drawn position came from. The two are different
 *  measurements of the same thing and a reader has to be told which one is
 *  under the mark: an oracle is exact by construction and an estimate is a
 *  solution with an accuracy of its own. */
export const OWNSHIP_SOURCE = Object.freeze({
  TRUTH: "simulation-truth",
  ESTIMATE: "operational-estimate",
});

/** How long a fix may go unrefreshed before the mark is withdrawn. A
 *  position that stopped arriving is a position the vehicle has left. */
export const OWNSHIP_STALE_AFTER_MS = 3_000;

/**
 * The vehicle position a telemetry sample states, or a typed reason it
 * states none.
 *
 * Two lanes can carry a position. The simulator's oracle states where the
 * vehicle IS; the flight controller's receiver states where it SOLVED that
 * it is. The oracle wins where a session has one, because a session that
 * has one is being judged against it — but a session without one is the
 * normal case, not a failure, and it is the only case a physical vehicle
 * has. The mark says which measurement is under it either way.
 *
 * Which lane a group belongs to is settled in the decoder, which refuses a
 * group stamped with another role outright, so a mislabelled group arrives
 * here as no group at all. This side does not repeat that gate: it would
 * name a reason the decoder makes unreachable, and a reason the client
 * cannot produce is a reason that lies when a reader meets it.
 */
export function ownshipFromTelemetry(telemetry) {
  const truth = telemetry?.simTruth;
  const avionics = telemetry?.avionics;
  if (!truth && !avionics) {
    return { position: null, source: null, reason: OWNSHIP_REASON.NO_SAMPLE };
  }
  const [fix, source] = truth?.geodetic
    ? [truth.geodetic, OWNSHIP_SOURCE.TRUTH]
    : [avionics?.geodetic, OWNSHIP_SOURCE.ESTIMATE];
  if (!fix) return { position: null, source: null, reason: OWNSHIP_REASON.NO_FIX };
  return {
    position: {
      latitudeDeg: fix.latitudeDeg,
      longitudeDeg: fix.longitudeDeg,
      heightM: fix.heightM,
    },
    source,
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
    // A position or a source beside an absent mark is something a reader
    // can still read off the surface.
    delete surface.dataset.ownshipPosition;
    delete surface.dataset.ownshipSource;
  };

  // Why the last sample carried no position, so a withdrawal can say which
  // silence it is: telemetry that stopped, or telemetry that kept arriving
  // and stopped carrying a fix. Reported as one reason they are the same to
  // a reader, and they are not the same thing at all.
  let lastAbsence = OWNSHIP_REASON.STOPPED;

  const age = (nowMs) => {
    if (lastFixAt === null) return;
    if (nowMs - lastFixAt <= OWNSHIP_STALE_AFTER_MS) return;
    withdraw(lastAbsence);
  };

  const observe = (telemetry, nowMs) => {
    const { position, source, reason } = ownshipFromTelemetry(telemetry);
    if (position === null) {
      lastAbsence = reason;
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
    // Telemetry stopping is the absence that follows a fix, until a sample
    // says otherwise.
    lastAbsence = OWNSHIP_REASON.STOPPED;
    marker.setLngLat([position.longitudeDeg, position.latitudeDeg]);
    if (!shown) {
      marker.addTo(map);
      shown = true;
    }
    const next =
      `Vehicle at ${position.latitudeDeg.toFixed(5)}, ${position.longitudeDeg.toFixed(5)}` +
      ` from the ${source === OWNSHIP_SOURCE.TRUTH ? "simulator" : "flight controller"}`;
    // An accessible name rewritten at the telemetry rate is announced at
    // the telemetry rate.
    if (next !== label) {
      element.setAttribute("aria-label", next);
      label = next;
    }
    surface.dataset.ownship = "shown";
    surface.dataset.ownshipSource = source;
    surface.dataset.ownshipPosition =
      `${position.latitudeDeg.toFixed(6)},${position.longitudeDeg.toFixed(6)}`;
    delete surface.dataset.ownshipReason;
  };

  withdraw(OWNSHIP_REASON.NO_SAMPLE);
  return { observe, age, marker };
}
