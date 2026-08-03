// The navigation display profile (ADR-0031): canonical wire units in,
// instrument-model display vocabulary out. The deflection signs are the
// safety-relevant part — a mirrored CDI flies the crew to the wrong side of
// course (FHA FC-HDG-03) — so both are pinned against the panel geometry
// that consumes them.
//
// Run: node clients/web/nav-display.test.mjs

import assert from "node:assert/strict";

import {
  LATERAL_M_PER_DOT,
  M_PER_NM,
  VDEV_M_PER_DOT,
  navDisplayState,
} from "./nav-display.js";

function snapshot(overrides = {}, ageMs = 40) {
  return {
    navGuidance: {
      toIdent: "WP02",
      fromIdent: "WP01",
      courseRad: 1.25,
      lateralDeviationM: 0,
      verticalDeviationM: 0,
      distanceToWaypointM: 3704,
      legIndex: 2,
      waypointCount: 5,
      solutionQuality: 0,
      ...overrides,
    },
    ageMs,
  };
}

function testEveryQuantityMapsIntoTheInstrumentVocabulary() {
  const nav = navDisplayState(snapshot({ lateralDeviationM: 50, verticalDeviationM: -16 }));
  assert.equal(nav.source, 1, "GPS/FMS course, drawn magenta");
  assert.equal(nav.fromto, 1, "flying toward the active waypoint");
  assert.equal(nav.courseRad, 1.25, "course passes through in radians");
  assert.equal(nav.courseReference, 1, "true north, the reference the wire declares");
  // Full-scale lateral is two dots at 25 m each: 50 m is exactly full scale.
  assert.equal(nav.cdiDots, -2);
  assert.equal(LATERAL_M_PER_DOT * 2, 50, "full-scale lateral deflection is ±50 m");
  // Full-scale vertical is two and a half dots at 8 m each: 16 m is two dots.
  assert.equal(nav.vdevDots, -2);
  assert.equal(VDEV_M_PER_DOT * 2.5, 20, "full-scale vertical deflection is ±20 m");
  assert.equal(nav.distNm, 3704 / M_PER_NM);
  assert.equal(nav.distNm, 2, "3704 m is exactly 2 NM");
  assert.equal(nav.ageMs, 40, "the freshness the tracker measured carries through");
}
testEveryQuantityMapsIntoTheInstrumentVocabulary();
console.log("ok - testEveryQuantityMapsIntoTheInstrumentVocabulary");

function testLateralDeflectionIsFlyTo() {
  // The panel draws the deviation bar at `cdi_dots * 37.5 px` in a course-up
  // frame where +x is the pilot's right, and the bar marks where the COURSE
  // is relative to ownship. The wire's cross-track deviation is positive
  // when ownship is right of course, so the course lies to ownship's LEFT
  // and the bar must deflect LEFT (negative) for the crew to fly toward it.
  const rightOfCourse = navDisplayState(snapshot({ lateralDeviationM: 25 }));
  assert.equal(rightOfCourse.cdiDots, -1, "ownship right of course deflects the bar left");

  const leftOfCourse = navDisplayState(snapshot({ lateralDeviationM: -25 }));
  assert.equal(leftOfCourse.cdiDots, 1, "ownship left of course deflects the bar right");

  // On course centers the bar, with the flag still showing TO.
  // Negating zero yields -0, which paints exactly where +0 does.
  const onCourse = navDisplayState(snapshot({ lateralDeviationM: 0 }));
  assert.ok(onCourse.cdiDots === 0, "on course centers the bar");
  assert.equal(onCourse.fromto, 1);

  // Beyond full scale the value is NOT clamped here: the panel owns its own
  // deflection limit, and clamping twice would hide a scaling error.
  assert.equal(navDisplayState(snapshot({ lateralDeviationM: 500 })).cdiDots, -20);
}
testLateralDeflectionIsFlyTo();
console.log("ok - testLateralDeflectionIsFlyTo");

function testVerticalDeflectionIsFlyTo() {
  // The panel draws the vertical pointer at `CY + vdev_dots * 38.4 px`, and
  // screen y grows DOWNWARD, so a positive value puts the pointer BELOW
  // center. The pointer marks where the profile is relative to ownship: the
  // wire's vertical deviation is positive when ownship is ABOVE the profile,
  // which puts the profile below ownship, so the deflection is POSITIVE and
  // the crew flies down toward it. The sign is not the mirror of the lateral
  // one — the two axes disagree on which way is up.
  const aboveProfile = navDisplayState(snapshot({ verticalDeviationM: 8 }));
  assert.equal(aboveProfile.vdevDots, 1, "above the profile puts the pointer below center");

  const belowProfile = navDisplayState(snapshot({ verticalDeviationM: -8 }));
  assert.equal(belowProfile.vdevDots, -1, "below the profile puts the pointer above center");

  assert.equal(navDisplayState(snapshot({ verticalDeviationM: 0 })).vdevDots, 0);
}
testVerticalDeflectionIsFlyTo();
console.log("ok - testVerticalDeflectionIsFlyTo");

function testUntrackedLateralCourseRemovesTheBarWithoutFailingTheGroup() {
  const nav = navDisplayState(snapshot({ lateralDeviationM: NaN }));
  // TO/FROM off is what removes the deviation bar; the deflection value is
  // then inert and must stay FINITE, because a NaN CDI fails the whole nav
  // group and would take the still-valid course and distance down with it.
  assert.equal(nav.fromto, 0, "no lateral geometry removes the deviation bar");
  assert.equal(nav.cdiDots, 0);
  assert.equal(Number.isFinite(nav.cdiDots), true, "an inert CDI is still a finite value");
  assert.equal(nav.courseRad, 1.25, "the course still displays");
  assert.equal(nav.distNm, 2, "the distance still displays");
}
testUntrackedLateralCourseRemovesTheBarWithoutFailingTheGroup();
console.log("ok - testUntrackedLateralCourseRemovesTheBarWithoutFailingTheGroup");

function testUnconstrainedVerticalProfileHasNoSample() {
  const nav = navDisplayState(snapshot({ verticalDeviationM: NaN }));
  // NaN is the instrument model's coding for "no sample"; the vertical
  // scale is not drawn at all, rather than drawn centered.
  assert.equal(Number.isNaN(nav.vdevDots), true);
  // The lateral guidance is untouched by the missing vertical constraint.
  assert.equal(nav.fromto, 1);
  assert.ok(nav.cdiDots === 0);
}
testUnconstrainedVerticalProfileHasNoSample();
console.log("ok - testUnconstrainedVerticalProfileHasNoSample");

function testUnusableSolutionRemovesTheGroupEntirely() {
  // ADR-0031: unusable removes the display. Returning a centered needle, or
  // guidance with a caution, would present a solution the client cannot
  // vouch for as flyable.
  assert.equal(navDisplayState(snapshot({ solutionQuality: 2 })), null);
  // Degraded still flies: it is a usable solution, displayed normally.
  assert.equal(navDisplayState(snapshot({ solutionQuality: 1 })).fromto, 1);
  // A quality coding this build does not know fails closed like unusable.
  assert.equal(navDisplayState(snapshot({ solutionQuality: 7 })), null);
}
testUnusableSolutionRemovesTheGroupEntirely();
console.log("ok - testUnusableSolutionRemovesTheGroupEntirely");

function testAbsentGuidanceIsAbsentNotCentered() {
  assert.equal(navDisplayState(null), null, "no accepted sample yet");
  assert.equal(navDisplayState({ navGuidance: null, ageMs: 10 }), null);
  // An unmeasurable age cannot feed the ABI's freshness slot, and guidance
  // with no freshness is not displayable.
  assert.equal(navDisplayState(snapshot({}, NaN)), null);
}
testAbsentGuidanceIsAbsentNotCentered();
console.log("ok - testAbsentGuidanceIsAbsentNotCentered");

console.log("\nall nav display profile checks passed");
