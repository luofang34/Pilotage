// The vehicle mark is drawn from a geodetic fix and from nothing else.
// Absence of a fix is absence of the mark, with a typed reason: a map that
// drew a vehicle at a default would tell a reader where the vehicle is
// when it does not know.
//
// Run: node clients/web/situation-ownship.test.mjs

import assert from "node:assert/strict";

import {
  OWNSHIP_REASON,
  OWNSHIP_SOURCE,
  OWNSHIP_STALE_AFTER_MS,
  attachOwnship,
  ownshipFromTelemetry,
} from "./situation-ownship.js";
import { LEADER_SECONDS } from "./situation-motion.js";

/** A body-FRD-to-NED rotation about the down axis: yaw alone. */
const yawQuat = (deg) => {
  const half = (deg * Math.PI) / 360;
  return { w: Math.cos(half), x: 0, y: 0, z: Math.sin(half) };
};

const VALID_ATTITUDE = 1;
const VALID_VELOCITY = 8;

const FIX = { latitudeDeg: 47.3977419, longitudeDeg: 8.5455938, heightM: 488.227 };
let tick = 0;
/** A stamp that differs from the last one, as a fresh measurement's does. */
const freshStamp = (role) => ({
  sourceId: 1,
  sourceIncarnation: "a",
  sourceEpoch: 1,
  sequence: (tick += 1),
  acquiredAtNanos: BigInt(tick) * 1_000_000n,
  role,
});

const truth = (overrides = {}) => ({
  simTruth: {
    stamp: freshStamp(2),
    geodetic: FIX,
    quat: yawQuat(90),
    velNed: [0, 10, 0],
    validFlags: VALID_ATTITUDE | VALID_VELOCITY,
    ...overrides,
  },
});

const estimate = (fix = FIX, overrides = {}) => ({
  avionics: {
    geodetic: fix,
    attitude: { quat: yawQuat(270) },
    kinematics: { velNed: [-10, 0, 0] },
    validFlags: VALID_ATTITUDE | VALID_VELOCITY,
    attitudeStamp: freshStamp(1),
    kinematicsStamp: freshStamp(1),
    geodeticStamp: freshStamp(1),
    estimatorStatusStamp: freshStamp(1),
    quality: 0,
    ...overrides,
  },
});

function testAFixDrawsTheVehicleWhereItSays() {
  const { position, source, reason } = ownshipFromTelemetry(truth());
  assert.equal(reason, null);
  assert.equal(source, OWNSHIP_SOURCE.TRUTH);
  assert.equal(position.latitudeDeg, FIX.latitudeDeg);
  assert.equal(position.longitudeDeg, FIX.longitudeDeg);
  assert.equal(position.heightM, FIX.heightM);
}
testAFixDrawsTheVehicleWhereItSays();
console.log("ok - testAFixDrawsTheVehicleWhereItSays");

function testNoFixDrawsNoVehicle() {
  for (const [label, sample, expected] of [
    ["no sample at all", null, OWNSHIP_REASON.NO_SAMPLE],
    ["a sample with no truth lane", {}, OWNSHIP_REASON.NO_SAMPLE],
    ["a truth lane with no fix", truth({ geodetic: null }), OWNSHIP_REASON.NO_FIX],
  ]) {
    const { position, reason } = ownshipFromTelemetry(sample);
    assert.equal(position, null, `${label} draws no vehicle`);
    assert.equal(reason, expected, `${label} says why`);
  }
}
testNoFixDrawsNoVehicle();
console.log("ok - testNoFixDrawsNoVehicle");

function testTheMarkNeverFallsBackToAnotherPosition() {
  // The failure this guards: a display that reached for the local frame,
  // or a last-known value, when the fix went absent. Both would draw a
  // vehicle the sample does not place.
  const withoutFix = ownshipFromTelemetry({
    simTruth: { stamp: { role: 2 }, geodetic: null, posNed: [10, 20, -30] },
  });
  assert.equal(withoutFix.position, null, "a local frame is not a position on the Earth");
}
testTheMarkNeverFallsBackToAnotherPosition();
console.log("ok - testTheMarkNeverFallsBackToAnotherPosition");

function testEitherLaneCanCarryThePositionAndTheMarkSaysWhich() {
  // A session with no truth oracle is the normal case, not a failure: it
  // is the only case a physical vehicle has, and an X-Plane session
  // flying a PX4 controller has it too.
  const fromEstimate = ownshipFromTelemetry(estimate());
  assert.equal(fromEstimate.position.latitudeDeg, FIX.latitudeDeg);
  assert.equal(fromEstimate.source, OWNSHIP_SOURCE.ESTIMATE);

  // Where a session has an oracle it is being judged against it, so the
  // oracle is what the mark shows.
  const both = { ...truth(), ...estimate({ ...FIX, latitudeDeg: 10 }) };
  const preferred = ownshipFromTelemetry(both);
  assert.equal(preferred.source, OWNSHIP_SOURCE.TRUTH);
  assert.equal(preferred.position.latitudeDeg, FIX.latitudeDeg);

  // An estimate lane with no fix is no fix, not a fallback to the frame.
  const neither = ownshipFromTelemetry({ avionics: { geodetic: null } });
  assert.equal(neither.position, null);
  assert.equal(neither.reason, OWNSHIP_REASON.NO_FIX);
}
testEitherLaneCanCarryThePositionAndTheMarkSaysWhich();
console.log("ok - testEitherLaneCanCarryThePositionAndTheMarkSaysWhich");

function testTheReasonsAreTheStringsAReaderMeets() {
  // These go on the surface and into the operator's own tooling, so they
  // are the observable contract and not an internal label.
  assert.deepEqual(OWNSHIP_REASON, {
    NO_SAMPLE: "OWNSHIP_NO_TELEMETRY",
    NO_FIX: "OWNSHIP_NO_FIX",
    STOPPED: "OWNSHIP_FIX_STOPPED",
  });
}
testTheReasonsAreTheStringsAReaderMeets();
console.log("ok - testTheReasonsAreTheStringsAReaderMeets");

// ---- the half that talks to a map ------------------------------------------

/** The smallest map and marker the module actually uses. */
function harness({ styleLoaded = true } = {}) {
  const events = [];
  const classes = new Set();
  const element = {
    classList: {
      toggle(name, force) {
        const on = force === undefined ? !classes.has(name) : Boolean(force);
        if (on) classes.add(name);
        else classes.delete(name);
        return on;
      },
      contains: (name) => classes.has(name),
    },
    get className() {
      return [...classes].join(" ");
    },
    set className(value) {
      classes.clear();
      for (const name of value.split(/\s+/).filter(Boolean)) classes.add(name);
    },
    attributes: {},
    setAttribute(name, value) {
      this.attributes[name] = value;
      events.push(`attr:${name}`);
    },
  };
  const surface = {
    dataset: {},
    ownerDocument: { createElement: () => element },
  };
  const leader = { data: null };
  const map = {
    id: "map",
    sources: new Map(),
    layers: [],
    isStyleLoaded: () => styleLoaded,
    pendingLoad: null,
    once(event, handler) {
      if (event === "load") this.pendingLoad = handler;
    },
    fireLoad() {
      this.pendingLoad?.();
    },
    addSource(name, spec) {
      this.sources.set(name, { ...spec, setData: (data) => (leader.data = data) });
    },
    getSource(name) {
      return this.sources.get(name);
    },
    addLayer(spec) {
      this.layers.push(spec);
    },
  };
  const marker = {
    lngLat: null,
    on: false,
    rotation: null,
    setRotation(value) {
      this.rotation = value;
      return this;
    },
    setLngLat(value) {
      this.lngLat = value;
      return this;
    },
    addTo(target) {
      assert.equal(target, map, "the mark joins the map it was attached to");
      this.on = true;
      events.push("add");
      return this;
    },
    remove() {
      this.on = false;
      events.push("remove");
      return this;
    },
  };
  let markerOptions = null;
  const maplibre = {
    Marker: function Marker(options) {
      markerOptions = options;
      options.element.classList.toggle("maplibregl-marker", true);
      return marker;
    },
  };
  const attached = attachOwnship(maplibre, map, surface);
  return { ...attached, surface, marker, element, events, leader, map, markerOptions };
}

function testAFixPutsTheMarkOnTheMap() {
  const { observe, surface, marker } = harness();
  assert.equal(surface.dataset.ownship, "absent", "no sample yet, no mark");
  assert.equal(surface.dataset.ownshipReason, OWNSHIP_REASON.NO_SAMPLE);

  observe(truth(), 1_000);

  assert.equal(marker.on, true, "the mark is on the map");
  assert.deepEqual(marker.lngLat, [FIX.longitudeDeg, FIX.latitudeDeg]);
  assert.equal(surface.dataset.ownship, "shown");
  assert.equal(surface.dataset.ownshipPosition, "47.397742,8.545594");
  assert.equal(surface.dataset.ownshipReason, undefined, "a shown mark states no reason");
  assert.equal(surface.dataset.ownshipSource, OWNSHIP_SOURCE.TRUTH, "and says which lane");
}
testAFixPutsTheMarkOnTheMap();
console.log("ok - testAFixPutsTheMarkOnTheMap");

function testAFixThatStopsArrivingWithdrawsTheMark() {
  // The failure this guards is the one a link going silent produces: no
  // further sample arrives, so nothing inside `observe` can ever run again,
  // and the mark would sit at its last position for as long as the page
  // stayed open.
  const { observe, age, surface, marker } = harness();
  observe(truth(), 1_000);
  assert.equal(marker.on, true);

  age(1_000 + OWNSHIP_STALE_AFTER_MS);
  assert.equal(marker.on, true, "inside the window the mark holds");
  assert.equal(surface.dataset.ownship, "shown");

  age(1_001 + OWNSHIP_STALE_AFTER_MS);
  assert.equal(marker.on, false, "past the window the mark is withdrawn");
  assert.equal(surface.dataset.ownship, "absent");
  assert.equal(
    surface.dataset.ownshipReason,
    OWNSHIP_REASON.STOPPED,
    "telemetry that stopped is not telemetry that carried no fix",
  );
  assert.equal(
    surface.dataset.ownshipPosition,
    undefined,
    "a position beside an absent mark is a position a reader can still read",
  );
  assert.equal(surface.dataset.ownshipSource, undefined);
}
testAFixThatStopsArrivingWithdrawsTheMark();
console.log("ok - testAFixThatStopsArrivingWithdrawsTheMark");

function testOneSampleWithoutAFixDoesNotBlinkTheMark() {
  const { observe, surface, marker } = harness();
  observe(truth(), 1_000);
  observe(truth({ geodetic: null }), 1_100);
  assert.equal(marker.on, true, "a single gap does not remove the mark");
  assert.equal(surface.dataset.ownship, "shown");

  observe(truth({ geodetic: null }), 1_001 + OWNSHIP_STALE_AFTER_MS);
  assert.equal(marker.on, false, "a gap past the window does");
  assert.equal(surface.dataset.ownship, "absent");
  assert.equal(
    surface.dataset.ownshipReason,
    OWNSHIP_REASON.NO_FIX,
    "telemetry that kept arriving without a fix says so",
  );
}
testOneSampleWithoutAFixDoesNotBlinkTheMark();
console.log("ok - testOneSampleWithoutAFixDoesNotBlinkTheMark");

function testTheAccessibleNameChangesOnlyWhenThePositionDoes() {
  // Telemetry arrives at the engine tick rate. An accessible name
  // rewritten at that rate is announced at that rate.
  const { observe, element, events } = harness();
  observe(truth(), 1_000);
  const afterFirst = events.filter((event) => event === "attr:aria-label").length;
  observe(truth(), 1_100);
  observe(truth(), 1_200);
  const afterRepeats = events.filter((event) => event === "attr:aria-label").length;
  assert.equal(afterFirst, 1, "the first fix names the mark");
  assert.equal(afterRepeats, 1, "an unchanged position renames nothing");

  observe({ simTruth: { stamp: { role: 2 }, geodetic: { ...FIX, latitudeDeg: 48.0 } } }, 1_300);
  assert.equal(
    events.filter((event) => event === "attr:aria-label").length,
    2,
    "a moved vehicle is renamed",
  );
  assert.match(element.attributes["aria-label"], /^Vehicle at 48\.00000, /);
}
testTheAccessibleNameChangesOnlyWhenThePositionDoes();
console.log("ok - testTheAccessibleNameChangesOnlyWhenThePositionDoes");

function testAWithdrawnMarkIsNotWithdrawnTwice() {
  const { observe, age, marker, events } = harness();
  observe(truth(), 1_000);
  age(9_000);
  age(9_500);
  age(10_000);
  assert.equal(marker.on, false);
  assert.equal(
    events.filter((event) => event === "remove").length,
    1,
    "an absent mark is not removed again on every tick",
  );
}
testAWithdrawnMarkIsNotWithdrawnTwice();
console.log("ok - testAWithdrawnMarkIsNotWithdrawnTwice");

function testTheEstimateLaneReachesTheMap() {
  // The pure decision function is tested above; this is the half that
  // talks to a map. A session with no truth oracle is the only case a
  // physical vehicle has, and until this ran, refusing the estimate lane
  // outright inside `observe` left every gate green.
  const { observe, surface, marker } = harness();
  observe(estimate(), 1_000);

  assert.equal(marker.on, true, "a receiver's fix puts the mark on the map");
  assert.deepEqual(marker.lngLat, [FIX.longitudeDeg, FIX.latitudeDeg]);
  assert.equal(surface.dataset.ownship, "shown");
  assert.equal(surface.dataset.ownshipSource, OWNSHIP_SOURCE.ESTIMATE);
}
testTheEstimateLaneReachesTheMap();
console.log("ok - testTheEstimateLaneReachesTheMap");

function testTheMarkSaysWhenTheLaneUnderItChanges() {
  // The two lanes are different measurements of the same thing, and the
  // mark can move several kilometres when it switches between them. The
  // surface has to say so on every sample, not only on the first.
  const { observe, surface, element } = harness();
  observe(truth(), 1_000);
  assert.equal(surface.dataset.ownshipSource, OWNSHIP_SOURCE.TRUTH);
  assert.match(element.attributes["aria-label"], /from the simulator,/);

  observe(estimate({ ...FIX, latitudeDeg: 47.5 }), 1_100);
  assert.equal(
    surface.dataset.ownshipSource,
    OWNSHIP_SOURCE.ESTIMATE,
    "the oracle went away and the mark says which measurement replaced it",
  );
  assert.match(element.attributes["aria-label"], /from the flight controller,/);

  observe(truth(), 1_200);
  assert.equal(
    surface.dataset.ownshipSource,
    OWNSHIP_SOURCE.TRUTH,
    "and says so again when the oracle returns",
  );
}
testTheMarkSaysWhenTheLaneUnderItChanges();
console.log("ok - testTheMarkSaysWhenTheLaneUnderItChanges");

function testTheMarkIsSomethingAReaderCanSee() {
  // The element is 0x0 and takes its whole shape from the class, so a mark
  // without it is "shown" on the surface and invisible on the screen. The
  // role is what the accessible name attaches to.
  const { observe, element } = harness();
  observe(truth(), 1_000);
  assert.ok(
    element.classList.contains("map-ownship"),
    "the mark has the shape the style draws",
  );
  assert.equal(element.attributes.role, "img", "the name has something to attach to");
}
testTheMarkIsSomethingAReaderCanSee();
console.log("ok - testTheMarkIsSomethingAReaderCanSee");

function testTheMarkTurnsToTheHeadingTheVehicleStates() {
  const { observe, surface, marker, markerOptions } = harness();
  observe(truth(), 1_000);

  assert.equal(marker.rotation, 90, "the mark turns to the stated yaw");
  assert.equal(surface.dataset.ownshipHeadingDeg, "90.0");
  // Both alignments are to the map: the map opens pitched and the reader
  // can turn it, and a mark aligned to the screen points somewhere the
  // vehicle is not for as long as either holds.
  assert.equal(markerOptions.rotationAlignment, "map");
  assert.equal(markerOptions.pitchAlignment, "map");
}
testTheMarkTurnsToTheHeadingTheVehicleStates();
console.log("ok - testTheMarkTurnsToTheHeadingTheVehicleStates");

function testAMarkWithNoStatedHeadingHasNoPointInIt() {
  // A reader reads a direction off a point. Rotating an unrotatable shape
  // to zero would state due north for a vehicle whose attitude nobody sent.
  const { observe, surface, marker, element } = harness();
  observe(truth({ validFlags: VALID_VELOCITY }), 1_000);

  assert.equal(surface.dataset.ownship, "shown", "the position is still known");
  assert.equal(surface.dataset.ownshipHeadingDeg, undefined, "and the heading is not");
  assert.equal(marker.rotation, 0);
  assert.match(element.className, /map-ownship-unknown-heading/);

  observe(truth(), 1_100);
  assert.ok(
    !element.classList.contains("map-ownship-unknown-heading"),
    "a stated heading restores the point",
  );
}
testAMarkWithNoStatedHeadingHasNoPointInIt();
console.log("ok - testAMarkWithNoStatedHeadingHasNoPointInIt");

function testTheLeaderReachesWhereTheVehicleArrives() {
  // The line is drawn in geographic coordinates, so its length is a
  // distance over the ground rather than a number of pixels: at 10 m/s the
  // minute ahead is 600 m due east of the fix.
  const { observe, leader } = harness();
  observe(truth(), 1_000);

  const [start, end] = leader.data.features[0].geometry.coordinates;
  assert.deepEqual(start, [FIX.longitudeDeg, FIX.latitudeDeg]);
  // The step is along a great circle, and a great circle leaving due east
  // is at its northernmost point, so the latitude falls away either side.
  // Over this minute that is under a metre.
  assert.ok(end[1] < FIX.latitudeDeg, "a great circle leaving east turns back down");
  assert.ok(Math.abs(end[1] - FIX.latitudeDeg) < 1e-5, `east endpoint latitude ${end[1]}`);
  const metres = 10 * LEADER_SECONDS;
  const expectedLon =
    FIX.longitudeDeg + metres / (111_111 * Math.cos((FIX.latitudeDeg * Math.PI) / 180));
  assert.ok(Math.abs(end[0] - expectedLon) < 1e-9, `east endpoint ${end[0]}`);
}
testTheLeaderReachesWhereTheVehicleArrives();
console.log("ok - testTheLeaderReachesWhereTheVehicleArrives");

function testTheLeaderFollowsTheTrackAndNotTheNose() {
  // In wind the two differ, and the difference is what a reader is
  // entitled to see: a leader drawn along the heading would hide it.
  const { observe, surface, marker, leader } = harness();
  // Nose due north, travelling due east.
  observe(truth({ quat: yawQuat(0), velNed: [0, 10, 0] }), 1_000);

  assert.equal(marker.rotation, 0, "the mark points where the nose does");
  assert.equal(surface.dataset.ownshipTrackDeg, "90.0", "the track is where it goes");
  const [, end] = leader.data.features[0].geometry.coordinates;
  assert.ok(end[0] > FIX.longitudeDeg, "the leader runs east, along the track");
  assert.ok(end[1] < FIX.latitudeDeg, "and not north, along the nose");
}
testTheLeaderFollowsTheTrackAndNotTheNose();
console.log("ok - testTheLeaderFollowsTheTrackAndNotTheNose");

function testAVehicleHoldingStationDrawsNoCourse() {
  const { observe, surface, leader } = harness();
  observe(truth({ velNed: [0, 0, 0] }), 1_000);

  assert.equal(leader.data.features.length, 0, "no line is drawn");
  assert.equal(surface.dataset.ownshipTrackDeg, undefined);
  assert.equal(surface.dataset.ownshipGroundSpeedMps, undefined);
  assert.equal(surface.dataset.ownship, "shown", "the position is still drawn");
}
testAVehicleHoldingStationDrawsNoCourse();
console.log("ok - testAVehicleHoldingStationDrawsNoCourse");

function testWithdrawingTheMarkTakesItsCourseWithIt() {
  const { observe, age, surface, leader, marker, element } = harness();
  observe(truth(), 1_000);
  assert.equal(leader.data.features.length, 1);

  age(1_001 + OWNSHIP_STALE_AFTER_MS);

  assert.equal(leader.data.features.length, 0, "a leader left drawn is a course still claimed");
  assert.equal(marker.rotation, 0);
  assert.match(element.className, /map-ownship-unknown-heading/);
  assert.equal(surface.dataset.ownshipHeadingDeg, undefined);
  assert.equal(surface.dataset.ownshipTrackDeg, undefined);
  assert.equal(surface.dataset.ownshipGroundSpeedMps, undefined);
}
testWithdrawingTheMarkTakesItsCourseWithIt();
console.log("ok - testWithdrawingTheMarkTakesItsCourseWithIt");

function testTheHeadingAndTheTrackComeFromTheLaneUnderTheMark() {
  // Position from the oracle turned by a heading from the estimate would
  // draw one measurement rotated by another, and nothing on the mark could
  // say so.
  const both = { ...truth(), ...estimate() };
  const chosen = ownshipFromTelemetry(both);
  assert.equal(chosen.source, OWNSHIP_SOURCE.TRUTH);
  assert.ok(Math.abs(chosen.headingDeg - 90) < 1e-6, "the truth lane's own yaw");
  assert.ok(Math.abs(chosen.track.bearingDeg - 90) < 1e-6, "the truth lane's own velocity");

  const estimateOnly = ownshipFromTelemetry(estimate());
  assert.equal(estimateOnly.source, OWNSHIP_SOURCE.ESTIMATE);
  assert.ok(Math.abs(estimateOnly.headingDeg - 270) < 1e-6, "the estimate lane's own yaw");
  assert.ok(Math.abs(estimateOnly.track.bearingDeg - 180) < 1e-6);
}
testTheHeadingAndTheTrackComeFromTheLaneUnderTheMark();
console.log("ok - testTheHeadingAndTheTrackComeFromTheLaneUnderTheMark");

function testTheLeaderWaitsForAStyleThatHasNotLoaded() {
  // `addSource` on a style that has not loaded throws, and the mark is
  // wired before the map reports load. The first line drawn before then is
  // the one drawn when it loads.
  const { observe, leader, map } = harness({ styleLoaded: false });
  observe(truth(), 1_000);
  assert.equal(leader.data, null, "nothing was drawn into a style with no source");

  map.fireLoad();
  assert.equal(leader.data.features.length, 1, "the held line is drawn once it can be");
}
testTheLeaderWaitsForAStyleThatHasNotLoaded();
console.log("ok - testTheLeaderWaitsForAStyleThatHasNotLoaded");

function testAMarkKeepsTheRendererIsOwnClass() {
  // The marker's placement is MapLibre's own class. Assigning the whole
  // class list to change the shape would take it away, and nothing puts it
  // back.
  const { observe, age, element } = harness();
  assert.ok(element.classList.contains("maplibregl-marker"), "the renderer claimed it");

  observe(truth(), 1_000);
  assert.ok(element.classList.contains("maplibregl-marker"), "and still has it when shown");
  age(1_001 + OWNSHIP_STALE_AFTER_MS);
  assert.ok(element.classList.contains("maplibregl-marker"), "and when withdrawn");
}
testAMarkKeepsTheRendererIsOwnClass();
console.log("ok - testAMarkKeepsTheRendererIsOwnClass");

function testADirectionWhoseGroupStoppedAdvancingIsNotDrawn() {
  // The estimate lane advances attitude, velocity and the fix apart, and
  // the producer withholds a group only after three seconds. A group that
  // is present in every sample and never advances is being republished,
  // not measured.
  const { observe, surface, leader, element } = harness();
  const stale = estimate();
  observe(stale, 1_000);
  assert.equal(surface.dataset.ownshipHeadingDeg, "270.0");
  assert.equal(surface.dataset.ownshipTrackDeg, "180.0");

  // The same groups again, with the fix moving on beneath them.
  const moved = {
    avionics: { ...stale.avionics, geodetic: { ...FIX, latitudeDeg: FIX.latitudeDeg + 0.001 } },
  };
  observe(moved, 1_000 + 200);
  assert.equal(surface.dataset.ownshipHeadingDeg, "270.0", "inside the limit it still counts");

  observe(moved, 1_000 + 400);
  assert.equal(surface.dataset.ownship, "shown", "the fix is still drawn");
  assert.equal(surface.dataset.ownshipHeadingDeg, undefined, "the nose is not");
  assert.equal(surface.dataset.ownshipTrackDeg, undefined);
  assert.equal(leader.data.features.length, 0, "and no course is left on the map");
  assert.ok(element.classList.contains("map-ownship-unknown-heading"));

  // A group that advances again is drawn again.
  const fresh = {
    avionics: { ...moved.avionics, attitudeStamp: freshStamp(1), kinematicsStamp: freshStamp(1) },
  };
  observe(fresh, 1_000 + 500);
  assert.equal(surface.dataset.ownshipHeadingDeg, "270.0", "a new measurement restores it");
}
testADirectionWhoseGroupStoppedAdvancingIsNotDrawn();
console.log("ok - testADirectionWhoseGroupStoppedAdvancingIsNotDrawn");

function testAHandoverBetweenLanesDoesNotInheritTheOtherLaneIsGroups() {
  // The lane is part of a stamp's identity, so the estimate lane's first
  // sample is a measurement this page has not seen however long the truth
  // lane had been repeating itself.
  const { observe, surface } = harness();
  observe(truth(), 1_000);
  assert.equal(surface.dataset.ownshipSource, OWNSHIP_SOURCE.TRUTH);

  const handover = estimate();
  observe(handover, 1_000 + 2_000);
  assert.equal(surface.dataset.ownshipSource, OWNSHIP_SOURCE.ESTIMATE);
  assert.equal(
    surface.dataset.ownshipHeadingDeg,
    "270.0",
    "the new lane's first measurement is current by definition",
  );
}
testAHandoverBetweenLanesDoesNotInheritTheOtherLaneIsGroups();
console.log("ok - testAHandoverBetweenLanesDoesNotInheritTheOtherLaneIsGroups");

function testACourseAlreadyDrawnIsHeldThroughTheFloor() {
  // Without a band between engaging and releasing, a vehicle drifting
  // either side of the floor flickers its course at the telemetry rate.
  const { observe, leader } = harness();
  observe(truth({ velNed: [0.45, 0, 0], stamp: freshStamp(2) }), 1_000);
  assert.equal(leader.data.features.length, 0, "below the floor no course starts");

  observe(truth({ velNed: [0.6, 0, 0], stamp: freshStamp(2) }), 1_100);
  assert.equal(leader.data.features.length, 1, "above the floor it does");

  observe(truth({ velNed: [0.45, 0, 0], stamp: freshStamp(2) }), 1_200);
  assert.equal(leader.data.features.length, 1, "and it is held back through the band");

  observe(truth({ velNed: [0.3, 0, 0], stamp: freshStamp(2) }), 1_300);
  assert.equal(leader.data.features.length, 0, "below the release speed it goes");
}
testACourseAlreadyDrawnIsHeldThroughTheFloor();
console.log("ok - testACourseAlreadyDrawnIsHeldThroughTheFloor");

function testTheCourseIsDrawnWhenTheSourceIsAlreadyOnTheStyle() {
  // A style that already carries the source is one to draw into. Treating
  // it as "not ready" would queue every line for the life of the page while
  // the mark went on turning.
  const { observe, leader, map } = harness({ styleLoaded: false });
  map.addSource("pilotage-ownship-leader", { type: "geojson", data: null });
  map.fireLoad();
  observe(truth(), 1_000);
  assert.equal(leader.data.features.length, 1, "the course reaches the source that was there");
}
testTheCourseIsDrawnWhenTheSourceIsAlreadyOnTheStyle();
console.log("ok - testTheCourseIsDrawnWhenTheSourceIsAlreadyOnTheStyle");

function testTheAccessibleNameNamesGroundSpeedAndNoBearingOf360() {
  const { observe, element } = harness();
  // 359.6 rounds to 360, which is a bearing no compass carries.
  observe(truth({ quat: yawQuat(359.6), velNed: [0, 10, 0], stamp: freshStamp(2) }), 1_000);
  const name = element.attributes["aria-label"];
  assert.match(name, /heading 0,/, `announced as ${name}`);
  assert.match(name, /metres per second over the ground$/, `announced as ${name}`);
}
testTheAccessibleNameNamesGroundSpeedAndNoBearingOf360();
console.log("ok - testTheAccessibleNameNamesGroundSpeedAndNoBearingOf360");

function testAGroupThatFrozeDoesNotComeBackFreshAfterAWithdrawal() {
  // The record of what each group last carried outlives the mark. A group
  // that had stopped advancing is still stopped when a fix returns, and
  // forgetting it here would draw the stale direction as a new one.
  const { observe, age, surface } = harness();
  const frozen = estimate();
  observe(frozen, 1_000);
  assert.equal(surface.dataset.ownshipHeadingDeg, "270.0");

  age(1_001 + OWNSHIP_STALE_AFTER_MS);
  assert.equal(surface.dataset.ownship, "absent");

  // The same groups, beside a fix that started arriving again.
  observe(frozen, 20_000);
  assert.equal(surface.dataset.ownship, "shown", "the fix is drawn again");
  assert.equal(
    surface.dataset.ownshipHeadingDeg,
    undefined,
    "the group that froze is still frozen",
  );
}
testAGroupThatFrozeDoesNotComeBackFreshAfterAWithdrawal();
console.log("ok - testAGroupThatFrozeDoesNotComeBackFreshAfterAWithdrawal");

function testTheEstimateLaneDrawsNoDirectionItWasNotAuthorizedToDraw() {
  // The mask and the quality beside it are a latched authorization from the
  // estimator, and both mean nothing without the status observation that
  // backs them. The map is the first consumer of this mask off the raw wire
  // message: the ingress the instruments read through applies the gate, and
  // the map is not handed its output.
  const unauthorized = ownshipFromTelemetry(estimate(FIX, { estimatorStatusStamp: null }));
  assert.ok(unauthorized.position, "the fix is a group of its own and still stands");
  assert.equal(unauthorized.headingDeg, null, "no authorization, no heading");
  assert.equal(unauthorized.track, null, "no authorization, no course");

  // An estimator that calls its own solution unusable has not authorized a
  // direction to be drawn from it either.
  const unusable = ownshipFromTelemetry(estimate(FIX, { quality: 2 }));
  assert.equal(unusable.headingDeg, null, "quality 2 is unusable");
  assert.equal(unusable.track, null);

  // Degraded is not unusable.
  const degraded = ownshipFromTelemetry(estimate(FIX, { quality: 1 }));
  assert.ok(Math.abs(degraded.headingDeg - 270) < 1e-6, "degraded is still a solution");

  // A mask absent altogether authorizes nothing.
  const noMask = ownshipFromTelemetry(estimate(FIX, { validFlags: undefined }));
  assert.equal(noMask.headingDeg, null, "an absent mask is not a full one");
  assert.equal(noMask.track, null);

  // The oracle has no estimator to authorize it, so its own mask stands.
  const oracle = ownshipFromTelemetry(truth());
  assert.ok(Math.abs(oracle.headingDeg - 90) < 1e-6, "truth needs no authorization");

  // A mask the oracle did not state is not a mask with every bit set. It
  // states which fields the sample carries, so absence is absence.
  const oracleNoMask = ownshipFromTelemetry(truth({ validFlags: undefined }));
  assert.ok(oracleNoMask.position, "the oracle's fix still stands");
  assert.equal(oracleNoMask.headingDeg, null, "an unstated mask states no availability");
  assert.equal(oracleNoMask.track, null);
}
testTheEstimateLaneDrawsNoDirectionItWasNotAuthorizedToDraw();
console.log("ok - testTheEstimateLaneDrawsNoDirectionItWasNotAuthorizedToDraw");

function testNoBearingIsEverStatedAs360() {
  // The two readers return a bearing in [0, 360). Formatting one decimal
  // place rounds 359.97 up, and a surface that reads "360.0" states a
  // bearing no compass carries.
  const { observe, surface } = harness();
  observe(truth({ quat: yawQuat(359.97), velNed: [10, -0.002, 0], stamp: freshStamp(2) }), 1_000);
  assert.equal(surface.dataset.ownshipHeadingDeg, "0.0", "the heading wraps, not rounds up");
  assert.equal(surface.dataset.ownshipTrackDeg, "0.0", "and so does the track");
}
testNoBearingIsEverStatedAs360();
console.log("ok - testNoBearingIsEverStatedAs360");

function testADirectionThatGoesAwayIsTakenOffTheSurface() {
  // A lane that loses its attitude bit in flight is the live case, and the
  // mark stays: only the direction goes. Nothing else clears the surface.
  const { observe, surface, leader, element } = harness();
  observe(truth(), 1_000);
  assert.equal(surface.dataset.ownshipHeadingDeg, "90.0");
  assert.equal(surface.dataset.ownshipTrackDeg, "90.0");

  observe(truth({ validFlags: 0, stamp: freshStamp(2) }), 1_100);
  assert.equal(surface.dataset.ownship, "shown", "the position is still stated");
  assert.equal(surface.dataset.ownshipHeadingDeg, undefined, "the heading is not");
  assert.equal(surface.dataset.ownshipTrackDeg, undefined);
  assert.equal(surface.dataset.ownshipGroundSpeedMps, undefined);
  assert.equal(leader.data.features.length, 0);
  assert.ok(element.classList.contains("map-ownship-unknown-heading"));
}
testADirectionThatGoesAwayIsTakenOffTheSurface();
console.log("ok - testADirectionThatGoesAwayIsTakenOffTheSurface");

console.log("\nall situation ownship checks passed");
