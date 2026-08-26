// The vehicle mark is drawn from a geodetic fix and from nothing else.
// Absence of a fix is absence of the mark, with a typed reason: a map that
// drew a vehicle at a default would tell a reader where the vehicle is
// when it does not know.
//
// Run: node clients/web/situation-ownship.test.mjs

import assert from "node:assert/strict";

import {
  OWNSHIP_REASON,
  OWNSHIP_STALE_AFTER_MS,
  attachOwnship,
  ownshipFromTelemetry,
} from "./situation-ownship.js";

const FIX = { latitudeDeg: 47.3977419, longitudeDeg: 8.5455938, heightM: 488.227 };
const truth = (overrides = {}) => ({
  simTruth: { stamp: { role: 2 }, geodetic: FIX, ...overrides },
});

function testAFixDrawsTheVehicleWhereItSays() {
  const { position, reason } = ownshipFromTelemetry(truth());
  assert.equal(reason, null);
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

function testTheReasonsAreTheStringsAReaderMeets() {
  // These go on the surface and into the operator's own tooling, so they
  // are the observable contract and not an internal label.
  assert.deepEqual(OWNSHIP_REASON, {
    NO_SAMPLE: "OWNSHIP_NO_TELEMETRY",
    NO_FIX: "OWNSHIP_NO_FIX",
  });
}
testTheReasonsAreTheStringsAReaderMeets();
console.log("ok - testTheReasonsAreTheStringsAReaderMeets");

// ---- the half that talks to a map ------------------------------------------

/** The smallest map and marker the module actually uses. */
function harness() {
  const events = [];
  const element = {
    className: "",
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
  const map = { id: "map" };
  const marker = {
    lngLat: null,
    on: false,
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
  const maplibre = { Marker: function Marker() { return marker; } };
  const attached = attachOwnship(maplibre, map, surface);
  return { ...attached, surface, marker, element, events };
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
  assert.equal(surface.dataset.ownshipReason, OWNSHIP_REASON.NO_FIX);
  assert.equal(
    surface.dataset.ownshipPosition,
    undefined,
    "a position beside an absent mark is a position a reader can still read",
  );
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

console.log("\nall situation ownship checks passed");
