// The geodetic fix on the wire (ADR-0022). A fix a reader cannot interpret
// is not a fix: the decoder returns null rather than a position, because a
// substituted or half-declared value draws a plausible vehicle in the wrong
// place. 0,0 is Null Island, a real place in the Gulf of Guinea.
//
// Run: node clients/web/geodetic-fix.test.mjs

import assert from "node:assert/strict";

import { decodeBareEnvelope } from "./wire.js";

const VARINT = 0;
const I64 = 1;
const LEN = 2;
const I32 = 5;

function varint(value) {
  const out = [];
  let rest = BigInt(value);
  do {
    let byte = Number(rest & 0x7fn);
    rest >>= 7n;
    if (rest > 0n) byte |= 0x80;
    out.push(byte);
  } while (rest > 0n);
  return out;
}

const tag = (field, kind) => varint((field << 3) | kind);

function double(field, value) {
  const buffer = new ArrayBuffer(8);
  new DataView(buffer).setFloat64(0, value, true);
  return [...tag(field, I64), ...new Uint8Array(buffer)];
}

const uint = (field, value) => [...tag(field, VARINT), ...varint(value)];
const nested = (field, body) => [...tag(field, LEN), ...varint(body.length), ...body];

function float(field, value) {
  const buffer = new ArrayBuffer(4);
  new DataView(buffer).setFloat32(0, value, true);
  return [...tag(field, I32), ...new Uint8Array(buffer)];
}

/** A MeasurementStamp the truth lane accepts: role 2 is simulation truth. */
function truthStamp(role = 2) {
  return [
    ...uint(1, 11), // source id
    ...uint(2, 1), // source epoch
    ...uint(3, 4), // sequence
    ...uint(4, 9_000_000), // acquired at, nanoseconds
    ...uint(5, 1), // clock
    ...nested(6, Array.from({ length: 16 }, () => 0x5a)), // incarnation
    ...uint(7, role), // 1 operational estimate, 2 simulation truth
    ...uint(8, 2), // integrity: checksummed only
  ];
}

/** A WGS-84 fix whose MSL height names the separation it came from. */
function wellFormedFix(overrides = {}) {
  const values = {
    latitudeDeg: 47.3977419,
    longitudeDeg: 8.5455938,
    horizontalDatum: 1,
    realization: 0,
    heightM: 488.227,
    verticalDatum: 2,
    geoidModel: 1,
    terrainRef: 0,
    baroSetting: 0,
    localOrigin: 0,
    horizontalAccuracyMm: 1500,
    verticalAccuracyMm: 3000,
    ...overrides,
  };
  return [
    ...double(1, values.latitudeDeg),
    ...double(2, values.longitudeDeg),
    ...uint(3, values.horizontalDatum),
    ...uint(4, values.realization),
    ...double(5, values.heightM),
    ...uint(6, values.verticalDatum),
    ...uint(7, values.geoidModel),
    ...uint(8, values.terrainRef),
    ...uint(9, values.baroSetting),
    ...uint(10, values.localOrigin),
    ...uint(11, values.horizontalAccuracyMm),
    ...uint(12, values.verticalAccuracyMm),
  ];
}

/** One TelemetrySample datagram carrying a SimTruthState. */
function truthDatagram({ fix = null, role = 2 } = {}) {
  const truth = [
    ...float(1, 1), // quat w
    ...float(2, 0),
    ...float(3, 0),
    ...float(4, 0),
    ...float(5, 0), // pos n
    ...float(6, 0),
    ...float(7, 0),
    ...float(8, 0), // vel n
    ...float(9, 0),
    ...float(10, 0),
    ...nested(11, truthStamp(role)),
    ...uint(12, 0b1101),
    ...(fix ? nested(13, fix) : []),
  ];
  const sample = [...uint(1, 1), ...uint(2, 1), ...nested(7, truth)];
  return Uint8Array.from(nested(4, sample));
}

const truthOf = (datagram) => {
  const decoded = decodeBareEnvelope(datagram);
  assert.equal(decoded.kind, "TelemetrySample");
  return decoded.message.simTruth;
};

function testAWellFormedFixCrosses() {
  const truth = truthOf(truthDatagram({ fix: wellFormedFix() }));
  assert.ok(truth, "the truth lane decodes");
  const fix = truth.geodetic;
  assert.ok(fix, "the fix decodes");
  assert.ok(Math.abs(fix.latitudeDeg - 47.3977419) < 1e-9);
  assert.ok(Math.abs(fix.longitudeDeg - 8.5455938) < 1e-9);
  assert.ok(Math.abs(fix.heightM - 488.227) < 1e-9);
  assert.equal(fix.horizontalDatum, 1, "WGS-84");
  assert.equal(fix.verticalDatum, 2, "MSL");
  assert.equal(fix.geoidModel, 1, "the separation is named");
  assert.equal(fix.horizontalAccuracyMm, 1500);
  assert.equal(fix.verticalAccuracyMm, 3000);
}
testAWellFormedFixCrosses();
console.log("ok - testAWellFormedFixCrosses");

function testAnAbsentFixIsAbsentNotNullIsland() {
  const truth = truthOf(truthDatagram());
  assert.ok(truth, "the truth lane still decodes without a fix");
  assert.equal(truth.geodetic, null, "no fix means no position");
  // The failure this guards: a reader that filled in zeros would place the
  // vehicle in the Gulf of Guinea and draw it as real. A present message
  // whose fields are all zero is the same failure wearing a wrapper, and
  // is refused for its unknown datum.
  const zeroed = truthOf(
    truthDatagram({ fix: wellFormedFix({
      latitudeDeg: 0, longitudeDeg: 0, heightM: 0,
      horizontalDatum: 0, verticalDatum: 0, geoidModel: 0,
    }) }),
  );
  assert.equal(zeroed.geodetic, null, "a zeroed fix is not a position");
  // An empty message carries no datum either.
  assert.equal(truthOf(truthDatagram({ fix: [] })).geodetic, null);
}
testAnAbsentFixIsAbsentNotNullIsland();
console.log("ok - testAnAbsentFixIsAbsentNotNullIsland");

function testAnUninterpretableDatumIsRefused() {
  const refused = {
    "an unknown horizontal datum": { horizontalDatum: 0 },
    "a horizontal datum this build does not know": { horizontalDatum: 9 },
    "NAD83 without a realization": { horizontalDatum: 2, realization: 0 },
    "an unknown vertical datum": { verticalDatum: 0 },
    "a vertical datum this build does not know": { verticalDatum: 9 },
    "an MSL height with no geoid model": { verticalDatum: 2, geoidModel: 0 },
    "an AGL height with no terrain reference": { verticalDatum: 3, terrainRef: 0 },
    "a barometric height with no applied setting": { verticalDatum: 4, baroSetting: 0 },
    "a local-relative height with no origin": { verticalDatum: 6, localOrigin: 0 },
    "a latitude past the pole": { latitudeDeg: 91 },
    "a longitude at the wrap point": { longitudeDeg: 180 },
    "a longitude past the wrap point": { longitudeDeg: -180.5 },
  };
  for (const [reason, overrides] of Object.entries(refused)) {
    const truth = truthOf(truthDatagram({ fix: wellFormedFix(overrides) }));
    assert.equal(truth.geodetic, null, `${reason} is refused`);
  }
  // A realization-bearing datum that DOES declare one is usable.
  const usable = truthOf(
    truthDatagram({ fix: wellFormedFix({ horizontalDatum: 2, realization: 7 }) }),
  );
  assert.ok(usable.geodetic, "NAD83 with a realization decodes");
  assert.equal(usable.geodetic.realization, 7);
}
testAnUninterpretableDatumIsRefused();
console.log("ok - testAnUninterpretableDatumIsRefused");

function testAFixInAMislabeledTruthLaneNeverArrives() {
  // A truth lane whose stamp claims the estimate role is mislabeled, and
  // the whole lane is unconsumable. A fix inside one must not survive it:
  // a simulator position that arrived as an operational estimate is
  // exactly the substitution the role gate exists to stop.
  const asEstimate = truthDatagram({ fix: wellFormedFix(), role: 1 });
  assert.equal(
    truthOf(asEstimate),
    null,
    "a truth lane stamped as an estimate is refused whole",
  );
  const asTruth = truthDatagram({ fix: wellFormedFix(), role: 2 });
  assert.ok(truthOf(asTruth).geodetic, "the same fix under the truth role arrives");
}
testAFixInAMislabeledTruthLaneNeverArrives();
console.log("ok - testAFixInAMislabeledTruthLaneNeverArrives");


/** One TelemetrySample datagram carrying an AvionicsState. */
function estimateDatagram({ fix = null, role = 1 } = {}) {
  const avionics = [
    ...float(1, 1), // quat w
    ...uint(14, 0b1111), // valid flags
    ...uint(15, 0), // quality
    ...nested(19, truthStamp(role)), // estimator status stamp
    ...(fix ? nested(22, fix) : []),
    ...nested(23, truthStamp(role)), // geodetic stamp
  ];
  const sample = [...uint(1, 1), ...uint(2, 1), ...nested(6, avionics)];
  return Uint8Array.from(nested(4, sample));
}

const estimateOf = (datagram) => {
  const decoded = decodeBareEnvelope(datagram);
  assert.equal(decoded.kind, "TelemetrySample");
  return decoded.message.avionics;
};

function testTheEstimateLaneTakesOnlyAnEstimateFix() {
  const own = estimateOf(estimateDatagram({ fix: wellFormedFix(), role: 1 }));
  assert.ok(own, "the estimate lane decodes");
  assert.ok(own.geodetic, "the estimator's own fix arrives");
  assert.ok(Math.abs(own.geodetic.latitudeDeg - 47.3977419) < 1e-9);

  // A simulator oracle's position placed in the estimate lane would read as
  // the estimator's own GNSS solution. Each lane gates on its own role.
  for (const wrong of [0, 2, 3, 5, 6, 99]) {
    const mislabeled = estimateOf(estimateDatagram({ fix: wellFormedFix(), role: wrong }));
    assert.equal(
      mislabeled.geodetic,
      null,
      `a fix stamped with role ${wrong} is not the estimator's own`,
    );
  }
}
testTheEstimateLaneTakesOnlyAnEstimateFix();
console.log("ok - testTheEstimateLaneTakesOnlyAnEstimateFix");

function testAnOriginKeepsItsIdentityPastFiftyThreeBits() {
  // Two origins that differ in their lowest bit, both past 2^53. Read as a
  // Number they compare equal, and an origin rebase — the one thing the
  // identity exists to make visible — becomes invisible.
  const first = 9_223_372_036_854_775_809n;
  const second = 9_223_372_036_854_775_810n;
  const withOrigin = (origin) =>
    truthOf(
      truthDatagram({
        fix: wellFormedFix({
          verticalDatum: 6, // local-relative
          geoidModel: 0,
          localOrigin: origin,
        }),
      }),
    ).geodetic;
  const a = withOrigin(first);
  const b = withOrigin(second);
  assert.ok(a && b, "a local-relative height with a declared origin decodes");
  assert.equal(a.localOrigin, first);
  assert.equal(b.localOrigin, second);
  assert.notEqual(a.localOrigin, b.localOrigin, "two origins stay two origins");
}
testAnOriginKeepsItsIdentityPastFiftyThreeBits();
console.log("ok - testAnOriginKeepsItsIdentityPastFiftyThreeBits");

function testAnIdentityPastItsTypeIsRefused() {
  // The producer's ids are u16 or u32. A wire value past that truncates on
  // the way back, so a height the typed contract refuses would be drawn.
  const refused = {
    "a realization past u16": { horizontalDatum: 2, realization: 70_000 },
    "a geoid model past u16": { geoidModel: 65_536 },
  };
  for (const [reason, overrides] of Object.entries(refused)) {
    const truth = truthOf(truthDatagram({ fix: wellFormedFix(overrides) }));
    assert.equal(truth.geodetic, null, `${reason} is refused`);
  }
}
testAnIdentityPastItsTypeIsRefused();
console.log("ok - testAnIdentityPastItsTypeIsRefused");

function testAnUnstatedAccuracyReadsAsUnstated() {
  // Proto3 omits a zero, so a producer that states nothing and one that
  // claims perfection send the same bytes. Neither may read as the best
  // possible fix.
  const fix = truthOf(
    truthDatagram({
      fix: wellFormedFix({ horizontalAccuracyMm: 0, verticalAccuracyMm: 0 }),
    }),
  ).geodetic;
  assert.ok(fix, "a fix with no stated accuracy is still a position");
  assert.equal(fix.horizontalAccuracyMm, null, "silence is not perfection");
  assert.equal(fix.verticalAccuracyMm, null);
  const stated = truthOf(truthDatagram({ fix: wellFormedFix() })).geodetic;
  assert.equal(stated.horizontalAccuracyMm, 1500);
}
testAnUnstatedAccuracyReadsAsUnstated();
console.log("ok - testAnUnstatedAccuracyReadsAsUnstated");

console.log("\nall geodetic fix checks passed");
