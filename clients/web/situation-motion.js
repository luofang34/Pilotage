// Where the vehicle points and where it is going: the two directions a
// moving map draws beside a position.
//
// They are not the same direction. Heading is where the nose points, from
// the attitude the vehicle states. Track is where the vehicle is actually
// travelling, from the velocity it states. In wind they differ by the crab
// angle, and that difference is information a reader is entitled to. A
// display that drew one and called it the other would be inventing an
// attitude nobody measured.

/** Below this ground speed a track direction is noise, not a bearing: a
 *  vehicle holding station reports a metre or two per second of drift whose
 *  direction wanders through the whole compass. Every display that draws a
 *  velocity leader suppresses it near zero, and drawing one here would be
 *  claiming a course the vehicle is not on. */
export const TRACK_FLOOR_MPS = 0.5;

/** The speed a leader already drawn must fall below before it is taken
 *  away. Without a band between the two, a vehicle drifting either side of
 *  the floor flickers its course on and off at the telemetry rate. */
export const TRACK_RELEASE_MPS = 0.35;

/** How far ahead the velocity leader reaches, in seconds. The line is the
 *  place the vehicle arrives at if it holds this velocity, so the duration
 *  is what makes its length mean anything; a leader with no stated
 *  look-ahead is a line of arbitrary length. Sixty seconds is the usual
 *  choice on a situation display. */
export const LEADER_SECONDS = 60;

/** Metres per degree of latitude, and of longitude at the equator. The
 *  same constant the link projects its local frame with, so a leader and a
 *  local frame do not disagree about the size of the Earth. */
const METRES_PER_DEGREE = 111_111;

/** The radius that constant implies, so the step below measures the same
 *  Earth the link projects against. */
const EARTH_RADIUS_M = (METRES_PER_DEGREE * 180) / Math.PI;

/** The validity bits a sample carries: bit 0 attitude, bit 3 velocity.
 *  A group whose bit is clear is a group the vehicle did not authorize,
 *  whatever numbers sit beside it. */
const VALID_ATTITUDE = 1;
const VALID_VELOCITY = 8;

/** A bearing wrapped into [0, 360). */
export const wrapBearingDeg = (deg) => ((deg % 360) + 360) % 360;

/**
 * The heading a sample states, in degrees clockwise from north, or `null`
 * when it states none.
 *
 * The quaternion rotates body FRD into world NED, so its yaw is the
 * heading. The north it is measured from is the world frame's; the
 * simulator worlds in use declare no heading offset, which makes it
 * geodetic north, and the wire states no heading reference for a consumer
 * to check that against.
 *
 * A quaternion that is not near unit length is not a rotation: a truncated
 * frame decodes to all zeros, which read as a rotation would give a
 * confident due north for a vehicle whose attitude nobody sent.
 *
 * The yaw's denominator is `w² + x² − y² − z²` rather than the `1 − 2(y² +
 * z²)` that assumes unit length. The two agree only at exactly unit
 * length, and the gate below passes a band either side of it — a
 * quaternion inside that band would otherwise decode to a heading wrong by
 * degrees, with the mark drawn pointed and its heading stated to a decimal
 * place. The form used here is exact at any scale.
 */
export function headingDegFrom(quat, validFlags) {
  if (!(validFlags & VALID_ATTITUDE)) return null;
  if (!quat) return null;
  const { w, x, y, z } = quat;
  if (![w, x, y, z].every(Number.isFinite)) return null;
  const normSquared = w * w + x * x + y * y + z * z;
  if (normSquared < 0.9 || normSquared > 1.1) return null;
  const yawRad = Math.atan2(2 * (w * z + x * y), w * w + x * x - y * y - z * z);
  return wrapBearingDeg((yawRad * 180) / Math.PI);
}

/**
 * The ground track a sample states — its bearing in degrees clockwise from
 * north and its speed in metres per second — or `null` when it states
 * none.
 *
 * `drawn` is whether a course is on the map already, which selects which
 * end of the hysteresis band applies.
 */
export function trackFrom(velNed, validFlags, drawn = false) {
  if (!(validFlags & VALID_VELOCITY)) return null;
  if (!Array.isArray(velNed) || velNed.length < 2) return null;
  const [north, east] = velNed;
  if (!Number.isFinite(north) || !Number.isFinite(east)) return null;
  const speedMps = Math.hypot(north, east);
  if (speedMps < (drawn ? TRACK_RELEASE_MPS : TRACK_FLOOR_MPS)) return null;
  const deg = (Math.atan2(east, north) * 180) / Math.PI;
  return { bearingDeg: wrapBearingDeg(deg), speedMps };
}

/**
 * The place the vehicle reaches by holding this velocity for
 * [`LEADER_SECONDS`], as `[longitude, latitude]`.
 *
 * The step is along a great circle. A flat step in degrees is accurate
 * enough over a minute in the middle latitudes, but it divides by the
 * cosine of the latitude, so near the pole it produces longitudes of
 * thousands of degrees and latitudes past 90 — places that are not on the
 * Earth. This form is defined everywhere, including across the pole and
 * across the antimeridian.
 *
 * The longitude returned is deliberately NOT wrapped into [-180, 180). The
 * leader is one two-vertex line and the renderer projects each vertex on
 * its own, so a pair either side of the antimeridian is drawn the long way
 * — westward across the whole world — in place of a line a few kilometres
 * long. What a renderer needs of the second vertex is that it lie within
 * 180 degrees of the first, which is what `atan2` returns. The wire's rule
 * that a longitude arrive normalized governs a position a producer states,
 * not an endpoint computed beside a start already known good.
 */
export function leaderEndpoint(position, track, seconds = LEADER_SECONDS) {
  const angular = (track.speedMps * seconds) / EARTH_RADIUS_M;
  const bearingRad = (track.bearingDeg * Math.PI) / 180;
  const latRad = (position.latitudeDeg * Math.PI) / 180;
  const lonRad = (position.longitudeDeg * Math.PI) / 180;
  const sinLat =
    Math.sin(latRad) * Math.cos(angular) +
    Math.cos(latRad) * Math.sin(angular) * Math.cos(bearingRad);
  // asin of a value a rounding error past 1 is NaN, and a NaN coordinate
  // takes the whole line off the map.
  const endLatRad = Math.asin(Math.min(1, Math.max(-1, sinLat)));
  const endLonRad =
    lonRad +
    Math.atan2(
      Math.sin(bearingRad) * Math.sin(angular) * Math.cos(latRad),
      Math.cos(angular) - Math.sin(latRad) * sinLat,
    );
  return [(endLonRad * 180) / Math.PI, (endLatRad * 180) / Math.PI];
}
