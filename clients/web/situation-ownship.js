// The vehicle's own position, heading and track on the situation map.
//
// The mark is drawn from a geodetic fix and from nothing else. Absence of a
// fix is absence of the mark, with the reason on the stage: a map that drew
// a vehicle at a default, at a last-known position, or at a place derived
// from a local origin would be telling a reader where the vehicle is when
// it does not know (ADR-0022, ADR-0037).
//
// The same rule governs the two directions beside it. The mark turns to the
// heading the vehicle states and carries a leader along the track it
// states; where it states neither, it is a shape with no direction in it
// and no line beside it. A pointed mark that never turned would assert a
// heading of screen-up on a map the reader can rotate.
//
// "States" means states now. Each direction has a group of its own that
// advances on its own, so a direction is drawn only while its group keeps
// producing measurements this page has not already seen.

import { headingDegFrom, leaderEndpoint, trackFrom } from "./situation-motion.js";

/** The map source and layer the velocity leader is drawn from. It is drawn
 *  in geographic coordinates, not in pixels: the line means "where the
 *  vehicle arrives in a minute", which is a distance over the ground, and a
 *  fixed pixel length would state a different distance at every zoom. */
const LEADER_SOURCE = "pilotage-ownship-leader";
const LEADER_LAYER = "pilotage-ownship-leader";

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

/** How long a direction's own group may go without a new measurement
 *  before the mark stops drawing it.
 *
 *  A group can be present in every sample and still be old: the estimate
 *  lane stamps attitude, velocity and the fix separately and advances them
 *  separately, and the producer withholds a group only after three seconds.
 *  At a routine yaw rate three seconds is most of a turn, so a nose drawn
 *  from a group that stopped advancing points somewhere the vehicle left.
 *  The instruments beside the map hold their groups to the same limit, and
 *  a heading they would already call incoherent has no business on the map
 *  as a measurement. */
export const GROUP_COHERENCE_MS = 300;

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
export function ownshipFromTelemetry(telemetry, { courseDrawn = false } = {}) {
  const truth = telemetry?.simTruth;
  const avionics = telemetry?.avionics;
  if (!truth && !avionics) {
    return { position: null, source: null, reason: OWNSHIP_REASON.NO_SAMPLE };
  }
  // One lane supplies all three. Position from the oracle beside a heading
  // from the estimate would draw one measurement turned by another, and
  // nothing on the mark could say so.
  const [lane, source] = truth?.geodetic
    ? [truth, OWNSHIP_SOURCE.TRUTH]
    : [avionics, OWNSHIP_SOURCE.ESTIMATE];
  const fix = lane?.geodetic;
  if (!fix) {
    return { position: null, source: null, reason: OWNSHIP_REASON.NO_FIX };
  }
  const validFlags = lane.validFlags ?? 0;
  return {
    position: {
      latitudeDeg: fix.latitudeDeg,
      longitudeDeg: fix.longitudeDeg,
      heightM: fix.heightM,
    },
    // The attitude group carries the quaternion on the estimate lane; the
    // truth lane carries it flat beside its own frame.
    headingDeg: headingDegFrom(lane.attitude?.quat ?? lane.quat, validFlags),
    track: trackFrom(lane.kinematics?.velNed ?? lane.velNed, validFlags, courseDrawn),
    // The stamp that governs each direction, so a caller can ask when its
    // group last advanced. The estimate lane stamps the two groups apart;
    // the truth lane states one observation and both ride it.
    headingStamp: lane.attitudeStamp ?? lane.stamp ?? null,
    trackStamp: lane.kinematicsStamp ?? lane.stamp ?? null,
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
  // Both alignments are to the MAP, not the viewport. The map opens
  // pitched and the reader can turn it, and a mark aligned to the screen
  // would point somewhere the vehicle is not for as long as either holds.
  const marker = new maplibre.Marker({
    element,
    rotationAlignment: "map",
    pitchAlignment: "map",
  });
  let shown = false;
  let lastFixAt = null;
  let label = null;
  let leaderReady = false;
  let pendingLeader = null;
  let courseDrawn = false;

  // When each direction's group last carried a measurement this page had
  // not already seen. A group repeated unchanged is a group the producer
  // is republishing, not measuring, and presence alone cannot tell the two
  // apart.
  const lastStampIdentity = new Map();
  const lastStampAdvance = new Map();
  const groupIsCurrent = (kind, stamp, nowMs) => {
    // A direction whose group states no stamp cannot be shown to be
    // current, and a direction that cannot be shown current is not drawn.
    if (!stamp) return false;
    // The role is in the identity with the rest, so a handover between
    // lanes reads as the new measurement it is without needing the lane
    // remembered separately.
    const identity = [
      stamp.role,
      stamp.sourceId,
      stamp.sourceIncarnation,
      stamp.sourceEpoch,
      stamp.sequence,
      stamp.acquiredAtNanos,
    ].join("/");
    if (lastStampIdentity.get(kind) !== identity) {
      lastStampIdentity.set(kind, identity);
      lastStampAdvance.set(kind, nowMs);
      return true;
    }
    const advancedAt = lastStampAdvance.get(kind);
    return advancedAt !== undefined && nowMs - advancedAt <= GROUP_COHERENCE_MS;
  };

  const emptyLeader = { type: "FeatureCollection", features: [] };

  const drawLeader = (data) => {
    if (!leaderReady) {
      // A style that has not loaded has no source to add one to. The last
      // line drawn before then is the one drawn when it does.
      pendingLeader = data;
      return;
    }
    map.getSource(LEADER_SOURCE)?.setData(data);
  };

  const addLeaderLayer = () => {
    if (leaderReady) return;
    // A source already on the style is one to draw into, not one to add
    // again. Leaving `leaderReady` false here would queue every line into
    // `pendingLeader` for the life of the page while the mark went on
    // turning, and no course would ever be drawn.
    if (map.getSource(LEADER_SOURCE)) {
      leaderReady = true;
      return;
    }
    map.addSource(LEADER_SOURCE, { type: "geojson", data: emptyLeader });
    map.addLayer({
      id: LEADER_LAYER,
      type: "line",
      source: LEADER_SOURCE,
      layout: { "line-cap": "round" },
      paint: { "line-color": "#d5006d", "line-width": 2 },
    });
    leaderReady = true;
    if (pendingLeader !== null) {
      map.getSource(LEADER_SOURCE)?.setData(pendingLeader);
      pendingLeader = null;
    }
  };

  if (map.isStyleLoaded?.()) addLeaderLayer();
  else map.once("load", addLeaderLayer);

  const withdraw = (reason) => {
    if (shown) {
      marker.remove();
      shown = false;
    }
    lastFixAt = null;
    surface.dataset.ownship = "absent";
    surface.dataset.ownshipReason = reason;
    // Anything beside an absent mark is something a reader can still read
    // off the surface, and a leader left drawn is a course still claimed.
    delete surface.dataset.ownshipPosition;
    delete surface.dataset.ownshipSource;
    delete surface.dataset.ownshipHeadingDeg;
    delete surface.dataset.ownshipTrackDeg;
    delete surface.dataset.ownshipGroundSpeedMps;
    marker.setRotation(0);
    // Assigning the whole class list would take MapLibre's own
    // `maplibregl-marker` class with it, and the marker's placement is
    // that class's rules.
    element.classList.toggle("map-ownship-unknown-heading", true);
    drawLeader(emptyLeader);
    courseDrawn = false;
    // The stamps are deliberately NOT forgotten. A group that had stopped
    // advancing is still stopped when the mark comes back, and clearing
    // the record here would let it return as though it were new.
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
    const sample = ownshipFromTelemetry(telemetry, { courseDrawn });
    const { position, source, reason } = sample;
    const headingDeg = groupIsCurrent("heading", sample.headingStamp, nowMs)
      ? sample.headingDeg
      : null;
    const track = groupIsCurrent("track", sample.trackStamp, nowMs) ? sample.track : null;
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
    // A shape with a point in it states a direction whether or not one was
    // measured, so the shape itself changes when the heading goes away.
    marker.setRotation(headingDeg ?? 0);
    element.classList.toggle("map-ownship-unknown-heading", headingDeg === null);
    if (!shown) {
      marker.addTo(map);
      shown = true;
    }
    courseDrawn = track !== null;
    drawLeader(
      track === null
        ? emptyLeader
        : {
            type: "FeatureCollection",
            features: [
              {
                type: "Feature",
                properties: {},
                geometry: {
                  type: "LineString",
                  coordinates: [
                    [position.longitudeDeg, position.latitudeDeg],
                    leaderEndpoint(position, track),
                  ],
                },
              },
            ],
          },
    );
    const next =
      `Vehicle at ${position.latitudeDeg.toFixed(5)}, ${position.longitudeDeg.toFixed(5)}` +
      ` from the ${source === OWNSHIP_SOURCE.TRUTH ? "simulator" : "flight controller"}` +
      // Whole degrees and whole metres per second: the name is announced
      // when it changes, and a fractional bearing changes every sample.
      // Rounded first and wrapped after: 359.6 rounds to 360, and a mark
      // announced as "heading 360" states a bearing no compass carries.
      (headingDeg === null
        ? ", heading unknown"
        : `, heading ${Math.round(headingDeg) % 360}`) +
      (track === null
        ? ", not tracking"
        : `, tracking ${Math.round(track.bearingDeg) % 360}` +
          ` at ${Math.round(track.speedMps)} metres per second over the ground`);
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
    if (headingDeg === null) delete surface.dataset.ownshipHeadingDeg;
    else surface.dataset.ownshipHeadingDeg = headingDeg.toFixed(1);
    if (track === null) {
      delete surface.dataset.ownshipTrackDeg;
      delete surface.dataset.ownshipGroundSpeedMps;
    } else {
      surface.dataset.ownshipTrackDeg = track.bearingDeg.toFixed(1);
      // Ground speed: the vertical component is deliberately not in it, and
      // the name has to say so or a reader takes it for speed through the
      // air or along the flight path.
      surface.dataset.ownshipGroundSpeedMps = track.speedMps.toFixed(2);
    }
    delete surface.dataset.ownshipReason;
  };

  withdraw(OWNSHIP_REASON.NO_SAMPLE);
  return { observe, age, marker };
}
