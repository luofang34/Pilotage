// Where the situation map's camera is pointing, and the controls that undo
// it. The vocabulary mirrors the Apple client's SituationCamera and
// MapControlsView so a reader meets the same rule on both clients: a
// control appears only when there is something to undo, because a control
// that does nothing teaches a reader to ignore the corner it sits in.
//
// The renderers report a bearing differently — MapLibre GL JS normalizes to
// (-180, 180], MapLibre Native reports 0 to 360 — so the heading is
// normalized here and the shared threshold is applied to the same quantity
// on both sides.

/** Turned far enough off north for a reader to notice. A fraction of a
 *  degree is a rounding artefact of a pinch, not a decision. */
const ROTATION_NOTICEABLE_DEG = 0.5;
/** Tilted away from straight down by more than a rounding artefact. */
const TILT_NOTICEABLE_DEG = 0.5;

/** Clockwise degrees away from north, normalized to [0, 360). */
export function normalizeHeadingDeg(bearingDeg) {
  if (!Number.isFinite(bearingDeg)) return 0;
  return ((bearingDeg % 360) + 360) % 360;
}

/** A camera reading: what the reader can undo, and whether there is
 *  anything to undo. */
export function situationCamera({ bearingDeg, pitchDeg }) {
  const headingDegrees = normalizeHeadingDeg(bearingDeg);
  const pitchDegrees = Number.isFinite(pitchDeg) ? pitchDeg : 0;
  return Object.freeze({
    headingDegrees,
    pitchDegrees,
    isRotated:
      headingDegrees > ROTATION_NOTICEABLE_DEG &&
      headingDegrees < 360 - ROTATION_NOTICEABLE_DEG,
    isTilted: pitchDegrees > TILT_NOTICEABLE_DEG,
  });
}

const COMPASS_POINTS = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
const SPOKEN = {
  N: "north",
  NE: "north east",
  E: "east",
  SE: "south east",
  S: "south",
  SW: "south west",
  W: "west",
  NW: "north west",
};

/** The compass point a heading rounds to. */
export function cardinal(headingDegrees) {
  const normalized = normalizeHeadingDeg(headingDegrees);
  return COMPASS_POINTS[Math.round(normalized / 45) % COMPASS_POINTS.length];
}

/** The same direction, said in full for a reader who cannot see it. */
export function spokenHeading(headingDegrees) {
  return SPOKEN[cardinal(headingDegrees)] ?? "north";
}

/** What the control that undoes a turn says. */
export function headingControlLabel(headingDegrees) {
  return `Facing ${spokenHeading(headingDegrees)}, turn back to north`;
}

/** What the control that undoes a tilt says. */
export const LEVEL_CONTROL_LABEL = "Look straight down";

/** How long a control's camera move takes, matching the Apple client. */
export const CAMERA_MOVE_DURATION_MS = 220;
