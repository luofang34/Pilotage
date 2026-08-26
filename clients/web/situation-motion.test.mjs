// Heading is where the nose points; track is where the vehicle goes. A
// display that drew one and called it the other would invent an attitude.
//
// Run: node clients/web/situation-motion.test.mjs

import assert from "node:assert/strict";

import {
  LEADER_SECONDS,
  TRACK_FLOOR_MPS,
  TRACK_RELEASE_MPS,
  headingDegFrom,
  leaderEndpoint,
  trackFrom,
} from "./situation-motion.js";

const VALID_ATTITUDE = 1;
const VALID_VELOCITY = 8;

/** A body-FRD-to-NED rotation about the down axis: yaw alone. */
const yawQuat = (deg) => {
  const half = (deg * Math.PI) / 360;
  return { w: Math.cos(half), x: 0, y: 0, z: Math.sin(half) };
};

const rad = (deg) => (deg * Math.PI) / 180;

/** A body-FRD-to-NED rotation with roll and pitch in it as well as yaw, so
 *  the yaw extraction is exercised on a quaternion whose x and y are not
 *  zero — the case where a denominator that assumes unit length goes
 *  wrong. */
const attitudeQuat = (rollDeg, pitchDeg, yawDeg) => {
  const [cr, sr] = [Math.cos(rad(rollDeg) / 2), Math.sin(rad(rollDeg) / 2)];
  const [cp, sp] = [Math.cos(rad(pitchDeg) / 2), Math.sin(rad(pitchDeg) / 2)];
  const [cy, sy] = [Math.cos(rad(yawDeg) / 2), Math.sin(rad(yawDeg) / 2)];
  return {
    w: cr * cp * cy + sr * sp * sy,
    x: sr * cp * cy - cr * sp * sy,
    y: cr * sp * cy + sr * cp * sy,
    z: cr * cp * sy - sr * sp * cy,
  };
};

const scaled = (quat, factor) => ({
  w: quat.w * factor,
  x: quat.x * factor,
  y: quat.y * factor,
  z: quat.z * factor,
});

function testHeadingReadsTheYawTheVehicleStates() {
  for (const deg of [0, 45, 90, 179, 270, 359]) {
    const read = headingDegFrom(yawQuat(deg), VALID_ATTITUDE);
    assert.ok(Math.abs(read - deg) < 1e-6, `${deg} reads back as ${read}`);
  }
  // Clockwise from north: east is 90, not -90.
  assert.ok(Math.abs(headingDegFrom(yawQuat(-90), VALID_ATTITUDE) - 270) < 1e-6);
}
testHeadingReadsTheYawTheVehicleStates();
console.log("ok - testHeadingReadsTheYawTheVehicleStates");

function testAnUnstatedAttitudeIsNoHeading() {
  // The bit is the vehicle's own authorization; numbers beside a clear bit
  // are numbers it did not stand behind.
  assert.equal(headingDegFrom(yawQuat(90), 0), null, "the attitude bit is clear");
  assert.equal(headingDegFrom(null, VALID_ATTITUDE), null, "no quaternion at all");

  // A truncated frame decodes to zeros, which is not a rotation. Read as
  // one it yields atan2(0, 1) — a confident due north for a vehicle whose
  // attitude nobody sent.
  assert.equal(
    headingDegFrom({ w: 0, x: 0, y: 0, z: 0 }, VALID_ATTITUDE),
    null,
    "a zero quaternion is not a heading of north",
  );
  assert.equal(
    headingDegFrom({ w: 5, x: 0, y: 0, z: 0 }, VALID_ATTITUDE),
    null,
    "a quaternion far off unit length is not a rotation",
  );
  assert.equal(
    headingDegFrom({ w: Number.NaN, x: 0, y: 0, z: 1 }, VALID_ATTITUDE),
    null,
    "a value that is not a number is not a heading",
  );
}
testAnUnstatedAttitudeIsNoHeading();
console.log("ok - testAnUnstatedAttitudeIsNoHeading");

function testTrackReadsTheVelocityTheVehicleStates() {
  // Due east at 10 m/s: north 0, east 10.
  const east = trackFrom([0, 10, 0], VALID_VELOCITY);
  assert.ok(Math.abs(east.bearingDeg - 90) < 1e-6);
  assert.ok(Math.abs(east.speedMps - 10) < 1e-6);

  // North-west: the bearing wraps into [0, 360).
  const northWest = trackFrom([10, -10, 0], VALID_VELOCITY);
  assert.ok(Math.abs(northWest.bearingDeg - 315) < 1e-6);
  assert.ok(Math.abs(northWest.speedMps - Math.hypot(10, 10)) < 1e-6);

  // The vertical component is not a ground track.
  const climbing = trackFrom([10, 0, -5], VALID_VELOCITY);
  assert.ok(Math.abs(climbing.speedMps - 10) < 1e-6, "a climb is not ground speed");
}
testTrackReadsTheVelocityTheVehicleStates();
console.log("ok - testTrackReadsTheVelocityTheVehicleStates");

function testAVehicleHoldingStationIsOnNoCourse() {
  assert.equal(trackFrom([0, 0, 0], VALID_VELOCITY), null, "stationary is no bearing");
  assert.equal(
    trackFrom([TRACK_FLOOR_MPS - 0.01, 0, 0], VALID_VELOCITY),
    null,
    "drift below the floor is noise, not a course",
  );
  assert.ok(
    trackFrom([TRACK_FLOOR_MPS + 0.01, 0, 0], VALID_VELOCITY) !== null,
    "above the floor it is a course",
  );
  assert.equal(trackFrom([0, 10, 0], 0), null, "the velocity bit is clear");
  assert.equal(trackFrom([Number.NaN, 0, 0], VALID_VELOCITY), null);
}
testAVehicleHoldingStationIsOnNoCourse();
console.log("ok - testAVehicleHoldingStationIsOnNoCourse");

function testTheLeaderReachesWhereTheVehicleArrives() {
  const position = { latitudeDeg: 47.4, longitudeDeg: 8.55, heightM: 500 };

  // Due north at 20 m/s for a minute is 1200 m, which is 1200/111111 of a
  // degree of latitude and no change of longitude.
  const north = leaderEndpoint(position, { bearingDeg: 0, speedMps: 20 });
  assert.ok(Math.abs(north[0] - position.longitudeDeg) < 1e-9, "due north changes no longitude");
  assert.ok(Math.abs(north[1] - (position.latitudeDeg + 1200 / 111_111)) < 1e-9);

  // Due east at the same speed covers the same ground distance, which is
  // more degrees of longitude because they are shorter this far north.
  const east = leaderEndpoint(position, { bearingDeg: 90, speedMps: 20 });
  // A great circle leaving due east is at its northernmost point, so the
  // latitude falls away either side of it. Over a minute that is under a
  // metre, which is why a flat step is accurate enough in these latitudes
  // and is not what this function does.
  assert.ok(east[1] < position.latitudeDeg, "a great circle turns back down");
  assert.ok(Math.abs(east[1] - position.latitudeDeg) < 1e-5, `east latitude ${east[1]}`);
  const expectedLon =
    position.longitudeDeg + 1200 / (111_111 * Math.cos((position.latitudeDeg * Math.PI) / 180));
  assert.ok(Math.abs(east[0] - expectedLon) < 1e-9);
  assert.ok(
    east[0] - position.longitudeDeg > north[1] - position.latitudeDeg,
    "a degree of longitude is shorter than a degree of latitude at this latitude",
  );

  // The look-ahead is what gives the line its length.
  const half = leaderEndpoint(position, { bearingDeg: 0, speedMps: 20 }, LEADER_SECONDS / 2);
  assert.ok(
    Math.abs(half[1] - position.latitudeDeg - (north[1] - position.latitudeDeg) / 2) < 1e-9,
  );
}
testTheLeaderReachesWhereTheVehicleArrives();
console.log("ok - testTheLeaderReachesWhereTheVehicleArrives");

function testEveryLeaderEndsSomewhereOnTheEarth() {
  // A step taken in degrees divides by the cosine of the latitude, so near
  // the pole it names longitudes in the thousands and latitudes past 90 —
  // places that are not on the Earth. Every row here is one such case.
  const cases = [
    ["at the pole", { latitudeDeg: 90, longitudeDeg: 0 }, { bearingDeg: 90, speedMps: 20 }],
    ["beside the pole", { latitudeDeg: 89.9999, longitudeDeg: 10 }, { bearingDeg: 90, speedMps: 30 }],
    ["over the pole", { latitudeDeg: 89.999, longitudeDeg: 10 }, { bearingDeg: 0, speedMps: 100 }],
    ["at the south pole", { latitudeDeg: -90, longitudeDeg: 0 }, { bearingDeg: 0, speedMps: 20 }],
    ["across the antimeridian", { latitudeDeg: 51.9, longitudeDeg: 179.98 }, { bearingDeg: 90, speedMps: 250 }],
    ["the other way across it", { latitudeDeg: 51.9, longitudeDeg: -179.98 }, { bearingDeg: 270, speedMps: 250 }],
  ];
  for (const [name, position, track] of cases) {
    const [lon, lat] = leaderEndpoint(position, track);
    assert.ok(Number.isFinite(lon) && Number.isFinite(lat), `${name}: the endpoint is a place`);
    assert.ok(lat >= -90 && lat <= 90, `${name}: latitude ${lat} is on the Earth`);
    // The wire contract carries a normalized longitude and this repo's own
    // decoder refuses one outside the band rather than wrap it.
    assert.ok(lon >= -180 && lon < 180, `${name}: longitude ${lon} is in range`);
  }

  // Crossing the pole comes down the opposite meridian, which a step in
  // degrees cannot do at all: it walks off the top of the map instead.
  const [overLon, overLat] = leaderEndpoint(
    { latitudeDeg: 89.999, longitudeDeg: 10 },
    { bearingDeg: 0, speedMps: 100 },
  );
  assert.ok(Math.abs(overLon - -170) < 1e-6, `over the pole the meridian flips: ${overLon}`);
  assert.ok(overLat < 90 && overLat > 89.9, `and comes back down: ${overLat}`);
}
testEveryLeaderEndsSomewhereOnTheEarth();
console.log("ok - testEveryLeaderEndsSomewhereOnTheEarth");

function testAQuaternionInsideTheGateReadsTheYawItCarries() {
  // The gate passes a band either side of unit length. A denominator of
  // `1 - 2(y² + z²)` is only the rotation's at exactly unit length, so a
  // quaternion inside that band would decode to a heading wrong by
  // degrees, drawn pointed and stated to a decimal place.
  const quat = attitudeQuat(30, 70, 110);
  for (const normSquared of [0.9, 0.95, 1, 1.05, 1.1]) {
    const read = headingDegFrom(scaled(quat, Math.sqrt(normSquared)), VALID_ATTITUDE);
    assert.ok(
      Math.abs(read - 110) < 1e-6,
      `at squared norm ${normSquared} the yaw reads ${read}, not 110`,
    );
  }
  // Outside the band nothing is claimed at all.
  assert.equal(headingDegFrom(scaled(quat, Math.sqrt(0.85)), VALID_ATTITUDE), null);
  assert.equal(headingDegFrom(scaled(quat, Math.sqrt(1.15)), VALID_ATTITUDE), null);
}
testAQuaternionInsideTheGateReadsTheYawItCarries();
console.log("ok - testAQuaternionInsideTheGateReadsTheYawItCarries");

function testACourseAlreadyDrawnReleasesLowerThanItEngages() {
  // Without a band, a vehicle drifting either side of the floor flickers
  // its course on and off at the telemetry rate.
  assert.ok(TRACK_RELEASE_MPS < TRACK_FLOOR_MPS, "the band has width");
  const between = (TRACK_FLOOR_MPS + TRACK_RELEASE_MPS) / 2;
  assert.equal(trackFrom([between, 0, 0], VALID_VELOCITY, false), null, "it does not engage");
  assert.ok(trackFrom([between, 0, 0], VALID_VELOCITY, true) !== null, "and does not release");
  assert.equal(
    trackFrom([TRACK_RELEASE_MPS - 0.01, 0, 0], VALID_VELOCITY, true),
    null,
    "below the release speed a drawn course goes",
  );
}
testACourseAlreadyDrawnReleasesLowerThanItEngages();
console.log("ok - testACourseAlreadyDrawnReleasesLowerThanItEngages");

console.log("\nall situation motion checks passed");
