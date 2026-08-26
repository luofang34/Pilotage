// The web client and the iPad draw the same two directions beside the same
// aircraft, and neither would notice the other drifting. Both are held to one
// corpus of cases instead.
//
// Run: node clients/web/situation-motion.conformance.test.mjs

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { LEADER_SECONDS, TRACK_FLOOR_MPS, TRACK_RELEASE_MPS, leaderEndpoint } from "./situation-motion.js";

const corpus = JSON.parse(
  readFileSync(new URL("../situation-ownship-motion.corpus.json", import.meta.url), "utf-8"),
);

function testTheCorpusStatesThisClientsOwnConstants() {
  // The corpus carries the shared thresholds. If this client changes one without
  // the corpus, the iPad keeps the old one and the two displays disagree about
  // when a course appears.
  assert.equal(corpus.leaderSeconds, LEADER_SECONDS);
  assert.equal(corpus.trackFloorMps, TRACK_FLOOR_MPS);
  assert.equal(corpus.trackReleaseMps, TRACK_RELEASE_MPS);
}
testTheCorpusStatesThisClientsOwnConstants();
console.log("ok - testTheCorpusStatesThisClientsOwnConstants");

function testEveryLeaderInTheCorpusIsWhereThisClientPutsIt() {
  for (const leader of corpus.leaders) {
    const [lon, lat] = leaderEndpoint(
      { latitudeDeg: leader.latitudeDeg, longitudeDeg: leader.longitudeDeg },
      { bearingDeg: leader.bearingDeg, speedMps: leader.groundSpeedMps },
      leader.seconds,
    );
    assert.ok(
      Math.abs(lon - leader.endLongitudeDeg) < 1e-7,
      `${leader.name}: longitude ${lon} is not ${leader.endLongitudeDeg}`,
    );
    assert.ok(
      Math.abs(lat - leader.endLatitudeDeg) < 1e-7,
      `${leader.name}: latitude ${lat} is not ${leader.endLatitudeDeg}`,
    );
  }
}
testEveryLeaderInTheCorpusIsWhereThisClientPutsIt();
console.log("ok - testEveryLeaderInTheCorpusIsWhereThisClientPutsIt");

function testTheCorpusAnchorsAreWhatGeometrySays() {
  // Agreement between two clients is not correctness if the corpus itself is
  // wrong. These rows are computed from geometry rather than from either
  // client: a minute due north at 20 m/s is 1200 m, which is 1200/111111 of a
  // degree of latitude and no change of longitude at all.
  const north = corpus.leaders.find((row) => row.name === "due north at the equator");
  const expected = (north.groundSpeedMps * north.seconds) / 111_111;
  assert.ok(Math.abs(north.endLatitudeDeg - expected) < 1e-9, "a minute due north");
  assert.equal(north.endLongitudeDeg, north.longitudeDeg, "due north changes no longitude");

  // Crossing the pole comes down the opposite meridian.
  const pole = corpus.leaders.find((row) => row.name === "across the north pole");
  assert.ok(Math.abs(pole.endLongitudeDeg - (pole.longitudeDeg + 180)) < 1e-6, "over the pole");
  assert.ok(pole.endLatitudeDeg < 90, "and back down onto the Earth");

  // A seam crossing stays a few kilometres long rather than becoming a turn of
  // the Earth.
  const seam = corpus.leaders.find((row) => row.name === "eastbound at the seam");
  assert.ok(Math.abs(seam.endLongitudeDeg - seam.longitudeDeg) < 1, "the seam segment is short");
}
testTheCorpusAnchorsAreWhatGeometrySays();
console.log("ok - testTheCorpusAnchorsAreWhatGeometrySays");

console.log("\nall situation motion conformance checks passed");
