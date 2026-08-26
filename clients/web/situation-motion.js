// Where the vehicle points and where it is going: the two directions a
// moving map draws beside a position.
//
// They are not the same direction. Heading is where the nose points, from
// the attitude the vehicle states. Track is where the vehicle is actually
// travelling, from the velocity it states. In wind they differ by the crab
// angle, and that difference is information a reader is entitled to. A
// display that drew one and called it the other would be inventing an
// attitude nobody measured (ADR-0037).

/** Below this ground speed a track direction is noise, not a bearing: a
 *  vehicle holding station reports a metre or two per second of drift whose
 *  direction wanders through the whole compass. Every display that draws a
 *  velocity leader suppresses it near zero, and drawing one here would be
 *  claiming a course the vehicle is not on. */
export const TRACK_FLOOR_MPS = 0.5;

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

/** The validity bits a sample carries: bit 0 attitude, bit 3 velocity.
 *  A group whose bit is clear is a group the vehicle did not authorize,
 *  whatever numbers sit beside it. */
const VALID_ATTITUDE = 1;
const VALID_VELOCITY = 8;

/**
 * The heading a sample states, in degrees clockwise from true north, or
 * `null` when it states none.
 *
 * The quaternion rotates body FRD into world NED, so its yaw is the
 * heading. A quaternion that is not near unit length is not a rotation: a
 * truncated frame decodes to all zeros, which reads as `atan2(0, 1)` — a
 * confident heading of due north for a vehicle whose attitude nobody sent.
 */
export function headingDegFrom(quat, validFlags) {
  if (!(validFlags & VALID_ATTITUDE)) return null;
  if (!quat) return null;
  const { w, x, y, z } = quat;
  if (![w, x, y, z].every(Number.isFinite)) return null;
  const norm = w * w + x * x + y * y + z * z;
  if (norm < 0.9 || norm > 1.1) return null;
  const yawRad = Math.atan2(2 * (w * z + x * y), 1 - 2 * (y * y + z * z));
  const deg = (yawRad * 180) / Math.PI;
  return ((deg % 360) + 360) % 360;
}

/**
 * The ground track a sample states — its bearing in degrees clockwise from
 * true north and its speed in metres per second — or `null` when it states
 * none.
 */
export function trackFrom(velNed, validFlags) {
  if (!(validFlags & VALID_VELOCITY)) return null;
  if (!Array.isArray(velNed) || velNed.length < 2) return null;
  const [north, east] = velNed;
  if (!Number.isFinite(north) || !Number.isFinite(east)) return null;
  const speedMps = Math.hypot(north, east);
  if (speedMps < TRACK_FLOOR_MPS) return null;
  const deg = (Math.atan2(east, north) * 180) / Math.PI;
  return { bearingDeg: ((deg % 360) + 360) % 360, speedMps };
}

/**
 * The place the vehicle reaches by holding this velocity for
 * [`LEADER_SECONDS`], as `[longitude, latitude]`.
 *
 * The step is taken on a flat Earth. Over the minute this line covers, the
 * error against a great circle is far below the width of the line itself.
 */
export function leaderEndpoint(position, track, seconds = LEADER_SECONDS) {
  const metres = track.speedMps * seconds;
  const bearingRad = (track.bearingDeg * Math.PI) / 180;
  const latitudeDeg =
    position.latitudeDeg + (metres * Math.cos(bearingRad)) / METRES_PER_DEGREE;
  const scale = Math.cos((position.latitudeDeg * Math.PI) / 180);
  // At the pole a degree of longitude is no distance at all, and the step
  // would divide by zero. A vehicle there has no meaningful longitude to
  // step along, so the leader points along the meridian alone.
  const longitudeDeg =
    Math.abs(scale) < 1e-6
      ? position.longitudeDeg
      : position.longitudeDeg + (metres * Math.sin(bearingRad)) / (METRES_PER_DEGREE * scale);
  return [longitudeDeg, latitudeDeg];
}
