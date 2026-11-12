// The client's navigation display profile (ADR-0031) — a thin wrapper
// over the shared feeder core (#252). The one meters-to-dots conversion
// runs in indicate-instrument-feeder via the instrument wasm build; the
// constants are mirrored here for consumers that size expectations, and
// the wrapper restores the instrument model's NaN coding for an
// unconstrained vertical profile.

import { bindings } from "./feeder-wasm.js";

// Full-scale lateral deflection is ±2 dots, so ±2 dots = ±50 m of cross-track
// error — the terminal-area scale a small unmanned airframe is flown to. The
// feeder declares that scale on the nav group, and this value mirrors the
// feeder's terminal row.
export const LATERAL_M_PER_DOT = 25;
// Full-scale vertical deflection is ±2.5 dots, so ±2.5 dots = ±20 m off the
// vertical profile.
export const VDEV_M_PER_DOT = 8;
export const M_PER_NM = 1852;

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
  if (bindings === null) return null;
  if (!snapshot || !snapshot.navGuidance || !Number.isFinite(snapshot.ageMs)) return null;
  const g = snapshot.navGuidance;
  // A quality, sequencing or TO/FROM outside the schema's non-negative
  // integer coding removes the display before any coercion could make it
  // look presentable.
  for (const code of [g.solutionQuality, g.sequencing, g.toFrom]) {
    if (!Number.isInteger(code) || code < 0) return null;
  }
  const marshalled = {
    navGuidance: {
      toIdent: typeof g.toIdent === "string" ? g.toIdent : "",
      fromIdent: typeof g.fromIdent === "string" ? g.fromIdent : "",
      courseRad: Number(g.courseRad),
      lateralDeviationM: Number(g.lateralDeviationM),
      verticalDeviationM: Number(g.verticalDeviationM),
      distanceToWaypointM: Number(g.distanceToWaypointM),
      legIndex: g.legIndex >>> 0,
      waypointCount: g.waypointCount >>> 0,
      solutionQuality: g.solutionQuality >>> 0,
      sequencing: g.sequencing >>> 0,
      toFrom: g.toFrom >>> 0,
    },
    ageMs: snapshot.ageMs,
  };
  const out = bindings.feeder_nav_display_state(marshalled);
  if (out === null || out === undefined) return null;
  // The feeder already speaks the instrument model's vocabulary, the
  // scale, facility and sequencing included; the model codes "no vertical
  // sample" as NaN where the feeder has none.
  return { ...out, vdevDots: out.vdevDots ?? NaN };
}
