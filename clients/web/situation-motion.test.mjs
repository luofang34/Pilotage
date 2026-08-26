// Heading is where the nose points; track is where the vehicle goes. A
// display that drew one and called it the other would invent an attitude.
//
// Run: node clients/web/situation-motion.test.mjs

import assert from "node:assert/strict";

import {
  LEADER_SECONDS,
  TRACK_FLOOR_MPS,
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
  assert.ok(Math.abs(east[1] - position.latitudeDeg) < 1e-9, "due east changes no latitude");
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

function testAtThePoleTheLeaderRunsAlongTheMeridian() {
  // A degree of longitude is no distance at the pole, and the step would
  // divide by zero.
  const pole = { latitudeDeg: 90, longitudeDeg: 0, heightM: 0 };
  const [lon, lat] = leaderEndpoint(pole, { bearingDeg: 90, speedMps: 20 });
  assert.ok(Number.isFinite(lon) && Number.isFinite(lat), "the endpoint is a place");
  assert.equal(lon, pole.longitudeDeg);
}
testAtThePoleTheLeaderRunsAlongTheMeridian();
console.log("ok - testAtThePoleTheLeaderRunsAlongTheMeridian");

console.log("\nall situation motion checks passed");
