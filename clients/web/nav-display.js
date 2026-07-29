// The client's navigation display profile (ADR-0031): the one place where
// the wire's canonical guidance units become the instrument model's display
// vocabulary. The wire carries meters and radians; the HSI's CDI and vertical
// scale are calibrated in dots, and the meters-per-dot deflection is per
// airframe class, so it lives here as a named constant with tests rather than
// on the wire where it would bind every display to one policy.

// Full-scale lateral deflection is ±2 dots, so ±2 dots = ±50 m of cross-track
// error — the terminal-area scale a small unmanned airframe is flown to.
export const LATERAL_M_PER_DOT = 25;
// Full-scale vertical deflection is ±2.5 dots, so ±2.5 dots = ±20 m off the
// vertical profile.
export const VDEV_M_PER_DOT = 8;
export const M_PER_NM = 1852;

// Instrument-model codings (pilotage-instrument-state): NavSource,
// NavFromTo, and HeadingReference as the packed ABI encodes them.
const NAV_SOURCE_GPS = 1;
const FROMTO_OFF = 0;
const FROMTO_TO = 1;
const HEADING_REFERENCE_TRUE = 1;
// Solution qualities this build can present. Unusable, and any coding a
// later host introduces, remove the display rather than drawing guidance
// the client cannot vouch for.
const PRESENTABLE_QUALITIES = Object.freeze([0, 1]);

/**
 * Converts one accepted guidance snapshot into the instrument runtime's `nav`
 * group, or `null` when guidance must not display at all.
 *
 * `snapshot` is what `NavGuidanceTracker.snapshot()` returns:
 * `{ navGuidance, ageMs }`, or `null` before any accepted sample. Returning
 * `null` removes the whole group — the ADR-0031 contract that absent guidance
 * is displayed as absent, never as a centered needle.
 */
export function navDisplayState(snapshot) {
  if (!snapshot || !snapshot.navGuidance || !Number.isFinite(snapshot.ageMs)) return null;
  const guidance = snapshot.navGuidance;
  if (!PRESENTABLE_QUALITIES.includes(guidance.solutionQuality)) return null;

  // Guidance that tracks no lateral course has no cross-track geometry to
  // draw. Clearing the TO/FROM flag removes the deviation bar, and the
  // deflection value goes to a finite zero the panel never paints — the
  // instrument model requires a finite CDI value, and NaN would fail the
  // whole group including the course and distance that are still valid.
  const tracking = Number.isFinite(guidance.lateralDeviationM);
  return {
    source: NAV_SOURCE_GPS,
    fromto: tracking ? FROMTO_TO : FROMTO_OFF,
    courseRad: guidance.courseRad,
    // The wire's course is measured from true north; the rose's own
    // reference is the simulator's local true north, and both measure from
    // true, so the conversion needs no variation sample.
    courseReference: HEADING_REFERENCE_TRUE,
    cdiDots: tracking ? lateralDots(guidance.lateralDeviationM) : 0,
    vdevDots: verticalDots(guidance.verticalDeviationM),
    distNm: guidance.distanceToWaypointM / M_PER_NM,
    ageMs: snapshot.ageMs,
  };
}

// Fly-to convention. The panel draws the deviation bar at
// `cdi_dots * PX_PER_DOT` in a course-up frame where +x is the pilot's
// right, and the bar marks where the course line is relative to ownship.
// The wire's cross-track deviation is positive when ownship is RIGHT of
// course, which puts the course to ownship's LEFT — so the deflection is
// negative, and flying toward the bar closes the error.
function lateralDots(lateralDeviationM) {
  return -lateralDeviationM / LATERAL_M_PER_DOT;
}

// Fly-to convention, with the screen's downward y accounted for. The panel
// draws the vertical pointer at `CY + vdev_dots * PX_PER_DOT` where larger y
// is LOWER on the display, and the pointer marks where the profile is
// relative to ownship. The wire's vertical deviation is positive when
// ownship is ABOVE the profile, which puts the profile BELOW ownship — lower
// on the display — so the deflection is positive, and flying down toward the
// pointer closes the error. The sign is therefore NOT the mirror of the
// lateral one: both are fly-to, and the axes disagree on which way is up.
//
// An unconstrained vertical profile stays NaN, the instrument model's coding
// for a quantity with no sample, and the scale is not drawn.
function verticalDots(verticalDeviationM) {
  return Number.isFinite(verticalDeviationM) ? verticalDeviationM / VDEV_M_PER_DOT : NaN;
}
