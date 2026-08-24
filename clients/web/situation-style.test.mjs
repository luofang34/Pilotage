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
const coastlineTileJson = {
  tilejson: "3.0.0",
  tiles: ["coastline/{z}/{x}/{y}.pbf"],
  minzoom: 0,
  maxzoom: 15,
};
const terrainTileJson = {
  tilejson: "3.0.0",
  tiles: ["terrain/{z}/{x}/{y}.png"],
  minzoom: 0,
  maxzoom: 13,
};

function resolved(overrides = {}) {
  return resolveSituationStyle({
    template: sharedStyle,
    assetsBase: ASSETS_BASE,
    coastlineTileJson,
    terrainTileJson,
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
  assert.equal(coastline.maxzoom, 15);
  const terrain = style.sources["pilotage-terrain"];
  assert.deepEqual(terrain.tiles, [`${ASSETS_BASE}terrain/{z}/{x}/{y}.png`]);
  assert.equal(terrain.maxzoom, 13, "overzoom past the deepest tile is the renderer's");
  assert.equal(style.glyphs, `${ASSETS_BASE}fonts/{fontstack}/{range}.pbf`);
}
testTokensResolveToInlineTileSources();
console.log("ok - testTokensResolveToInlineTileSources");

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
      () => resolved({ coastlineTileJson: broken }),
      (error) =>
        error instanceof SituationStyleError &&
        error.reason === STYLE_REASON.INVALID_TILEJSON,
    );
  }
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
