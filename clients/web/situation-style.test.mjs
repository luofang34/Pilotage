// Situation style resolution, pinned against the REAL shared artifacts:
// the style template and the terrain manifest the Apple client ships, and
// the Swift resolver's own constants. One style file drives both renderers,
// so the parity checks read the Apple sources rather than copies — a value
// changed on one side fails here until the other side follows.
//
// Run: node clients/web/situation-style.test.mjs

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  COASTLINE_TOKEN,
  FALLBACK_STYLE,
  OVERZOOM_STEPS,
  STYLE_REASON,
  SituationStyleError,
  TERRAIN_TOKEN,
  deriveMaximumZoom,
  resolveSituationStyle,
  sourceAttributions,
} from "./situation-style.js";
import { INITIAL_CAMERA } from "./situation-map.js";

const webRoot = dirname(fileURLToPath(import.meta.url));
const appleResources = join(webRoot, "..", "apple", "Resources");
const appleApp = join(webRoot, "..", "apple", "App");

const sharedStyle = JSON.parse(
  readFileSync(join(appleResources, "SituationStyle.json"), "utf8"),
);
const terrainManifest = JSON.parse(
  readFileSync(join(appleResources, "SituationTerrain.manifest.json"), "utf8"),
);
const styleResolverSwift = readFileSync(
  join(appleApp, "SituationStyleResource.swift"),
  "utf8",
);
const mapDefaultsSwift = readFileSync(join(appleApp, "PilotageApp.swift"), "utf8");

const ASSETS_BASE = "http://client.test/situation-assets/";
const tileJsonBySource = {
  "pilotage-coastline": {
    tilejson: "3.0.0", tiles: ["coastline/{z}/{x}/{y}.pbf"], minzoom: 0, maxzoom: 7,
  },
  "pilotage-terrain": {
    tilejson: "3.0.0", tiles: ["terrain/{z}/{x}/{y}.png"], minzoom: 0, maxzoom: 13,
  },
};

function resolved(overrides = {}) {
  return resolveSituationStyle({
    template: sharedStyle,
    assetsBase: ASSETS_BASE,
    tileJsonBySource,
    glyphsPath: "fonts",
    ...overrides,
  });
}

function testTokensResolveToInlineTileSources() {
  const style = resolved();
  const coastline = style.sources["pilotage-coastline"];
  assert.equal(coastline.url, undefined, "the archive token is consumed");
  assert.deepEqual(coastline.tiles, [`${ASSETS_BASE}coastline/{z}/{x}/{y}.pbf`]);
  assert.equal(coastline.minzoom, 0);
  assert.equal(coastline.maxzoom, 7, "overzoom past the deepest tile is the renderer's");
  const terrain = style.sources["pilotage-terrain"];
  assert.deepEqual(terrain.tiles, [`${ASSETS_BASE}terrain/{z}/{x}/{y}.png`]);
  assert.equal(terrain.maxzoom, 13, "overzoom past the deepest tile is the renderer's");
  assert.equal(style.glyphs, `${ASSETS_BASE}fonts/{fontstack}/{range}.pbf`);
}
testTokensResolveToInlineTileSources();
console.log("ok - testTokensResolveToInlineTileSources");

function testTheCoastlineSourceStopsWhereTheArchiveIsStillGlobal() {
  // A vector tile the archive does not hold draws nothing, and no
  // shallower tile stands in for it the way a raster tile's parent does.
  // Past the deepest zoom a source declares, the renderer stretches the
  // tile it has instead of asking for one that does not exist. So the
  // depth the source declares has to be a depth the archive holds
  // EVERYWHERE, and the plan is what states that.
  const plan = JSON.parse(
    readFileSync(new URL("../apple/Resources/SituationCoastline.plan.json", import.meta.url)),
  );
  for (const band of plan.bands) {
    assert.ok(
      band.min_lon_deg <= -180 && band.max_lon_deg >= 180 &&
        band.min_lat_deg <= -85 && band.max_lat_deg >= 85,
      `band ${band.name} covers the world`,
    );
  }
  for (let zoom = 0; zoom <= plan.closest_zoom; zoom += 1) {
    const covering = plan.bands.filter(
      (band) => band.min_zoom <= zoom && band.max_zoom >= zoom,
    );
    assert.equal(covering.length, 1, `exactly one band covers zoom ${zoom}`);
  }
  assert.equal(
    plan.closest_zoom,
    Math.max(...plan.bands.map((band) => band.max_zoom)),
    "the closest zoom is the deepest band",
  );
  // The export writes the source's ceiling from the tiles the archive
  // holds, so the fixture above states what the plan promises. Reading
  // the export itself would make this suite need a build artifact.
  assert.equal(
    tileJsonBySource["pilotage-coastline"].maxzoom,
    plan.closest_zoom,
    "the fixture stops where the plan says the archive stops",
  );
}
testTheCoastlineSourceStopsWhereTheArchiveIsStillGlobal();
console.log("ok - testTheCoastlineSourceStopsWhereTheArchiveIsStillGlobal");

function testResolutionPreservesTheSharedContract() {
  const style = resolved();
  // The web renderer draws a globe from the shared style's projection key.
  assert.equal(style.projection?.type, "globe");
  const terrain = style.sources["pilotage-terrain"];
  assert.equal(terrain.type, "raster-dem");
  assert.equal(terrain.encoding, "terrarium");
  assert.equal(terrain.tileSize, 256);
  // Attribution is a licence condition of the tile sources; substitution
  // must never drop it.
  assert.deepEqual(sourceAttributions(style), sourceAttributions(sharedStyle));
  assert.equal(sourceAttributions(style).length, 2);
  // The template itself stays untouched: it is the shared document.
  assert.equal(sharedStyle.sources["pilotage-coastline"].url, COASTLINE_TOKEN);
  assert.equal(sharedStyle.sources["pilotage-terrain"].url, TERRAIN_TOKEN);
}
testResolutionPreservesTheSharedContract();
console.log("ok - testResolutionPreservesTheSharedContract");

function testMissingFontsCostTheLabelsNotTheMap() {
  const style = resolved({ glyphsPath: null });
  assert.equal(style.glyphs, undefined, "the glyphs key is dropped");
  assert.equal(style.layers.length, sharedStyle.layers.length, "every layer survives");
}
testMissingFontsCostTheLabelsNotTheMap();
console.log("ok - testMissingFontsCostTheLabelsNotTheMap");

function testInvalidTemplateIsATypedRefusal() {
  const forged = structuredClone(sharedStyle);
  forged.sources["pilotage-coastline"].url = "coastline.tilejson.json";
  assert.throws(
    () => resolved({ template: forged }),
    (error) =>
      error instanceof SituationStyleError &&
      error.reason === STYLE_REASON.INVALID_TEMPLATE,
  );
}
testInvalidTemplateIsATypedRefusal();
console.log("ok - testInvalidTemplateIsATypedRefusal");

function testUnusableTileJsonIsATypedRefusal() {
  for (const broken of [null, {}, { tiles: [] }, { tiles: ["a"], minzoom: 0 }]) {
    assert.throws(
      () =>
        resolved({
          tileJsonBySource: { ...tileJsonBySource, "pilotage-coastline": broken },
        }),
      (error) =>
        error instanceof SituationStyleError &&
        error.reason === STYLE_REASON.INVALID_TILEJSON,
    );
  }
  // A source the export does not describe is refused, never left empty.
  assert.throws(
    () => resolved({ tileJsonBySource: { "pilotage-coastline": tileJsonBySource["pilotage-coastline"] } }),
    (error) => error instanceof SituationStyleError,
  );
}
testUnusableTileJsonIsATypedRefusal();
console.log("ok - testUnusableTileJsonIsATypedRefusal");

function testMaximumZoomDerivesFromTheManifest() {
  // The committed manifest, exactly as the Apple resolver reads it.
  const deepest = Math.max(...terrainManifest.bands.map((band) => band.max_zoom));
  assert.equal(deriveMaximumZoom(terrainManifest), deepest + OVERZOOM_STEPS);
  // Unreadable manifest and missing bands mirror the Swift guard path.
  assert.equal(deriveMaximumZoom(null), 14);
  assert.equal(deriveMaximumZoom({}), 14);
  assert.equal(deriveMaximumZoom({ bands: "not-a-list" }), 14);
  // The Swift cast refuses the whole band list when one entry is not a
  // dictionary.
  assert.equal(deriveMaximumZoom({ bands: [null] }), 14);
  assert.equal(deriveMaximumZoom({ bands: [{ max_zoom: 13 }, 7] }), 14);
  // Bands without depths fall back to the Swift default deepest of 13.
  assert.equal(deriveMaximumZoom({ bands: [] }), 13 + OVERZOOM_STEPS);
  assert.equal(deriveMaximumZoom({ bands: [{}] }), 13 + OVERZOOM_STEPS);
}
testMaximumZoomDerivesFromTheManifest();
console.log("ok - testMaximumZoomDerivesFromTheManifest");

function testFallbackStyleMatchesTheAppleResolver() {
  const literal = styleResolverSwift.match(
    /static let fallbackJSON = """\s*\n(.*)\n\s*"""/,
  );
  assert.ok(literal, "SituationStyleResource.swift carries the fallback literal");
  assert.deepEqual(FALLBACK_STYLE, JSON.parse(literal[1].trim()));
}
testFallbackStyleMatchesTheAppleResolver();
console.log("ok - testFallbackStyleMatchesTheAppleResolver");

function testOverzoomMatchesTheAppleResolver() {
  const steps = styleResolverSwift.match(/overzoomSteps: Double = ([\d.]+)/);
  assert.ok(steps, "SituationStyleResource.swift declares overzoomSteps");
  assert.equal(OVERZOOM_STEPS, Number(steps[1]));
}
testOverzoomMatchesTheAppleResolver();
console.log("ok - testOverzoomMatchesTheAppleResolver");

function testInitialCameraMatchesTheAppleClient() {
  const pitch = mapDefaultsSwift.match(/initialPitchDegrees = (-?[\d.]+)/);
  const center = mapDefaultsSwift.match(
    /initialCenter = CLLocationCoordinate2D\(latitude: (-?[\d.]+), longitude: (-?[\d.]+)\)/,
  );
  const zoom = mapDefaultsSwift.match(/initialZoomLevel = (-?[\d.]+)/);
  const minimum = mapDefaultsSwift.match(/minimumZoomLevel = (-?[\d.]+)/);
  assert.ok(pitch && center && zoom && minimum, "PilotageApp.swift declares the camera");
  assert.equal(INITIAL_CAMERA.pitchDegrees, Number(pitch[1]));
  assert.equal(INITIAL_CAMERA.centerLatitudeDeg, Number(center[1]));
  assert.equal(INITIAL_CAMERA.centerLongitudeDeg, Number(center[2]));
  assert.equal(INITIAL_CAMERA.zoomLevel, Number(zoom[1]));
  assert.equal(INITIAL_CAMERA.minimumZoomLevel, Number(minimum[1]));
}
testInitialCameraMatchesTheAppleClient();
console.log("ok - testInitialCameraMatchesTheAppleClient");

console.log("\nall situation style checks passed");
