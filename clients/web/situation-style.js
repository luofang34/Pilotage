// Situation style resolution for the web renderer.
//
// One style file drives the Apple renderer and the web renderer
// (clients/apple/Resources/SituationStyle.json, exported verbatim by
// scripts/build-web-situation-assets.sh). This module substitutes the
// style's three __PILOTAGE_*__ URL tokens for the exported asset URLs,
// mirroring the Apple client's SituationStyleResource: an invalid template
// is a typed error, a missing font set costs the labels and never the map,
// and the closest zoom derives from the terrain manifest rather than a
// second hand-written number.

export const GLYPHS_TOKEN = "__PILOTAGE_GLYPHS_URL__";
export const COASTLINE_TOKEN = "__PILOTAGE_COASTLINE_MBTILES_URL__";
export const TERRAIN_TOKEN = "__PILOTAGE_TERRAIN_MBTILES_URL__";

/** Zoom levels the camera may go past the deepest terrain tile. A
 *  raster-dem source draws past its deepest tile by stretching the one it
 *  has, and far past it the picture is invention. The value matches the
 *  Apple client's overzoomSteps, so both renderers stop at the same
 *  closest zoom. */
export const OVERZOOM_STEPS = 2;

/** The style shown when the situation style cannot be resolved. Matches the
 *  Apple client's fallbackJSON: a plain background, no source, no layer. */
export const FALLBACK_STYLE = Object.freeze({
  version: 8,
  name: "Pilotage terrain unavailable",
  sources: {},
  layers: [
    { id: "background", type: "background", paint: { "background-color": "#0b1721" } },
  ],
});

/** A typed situation-style failure. `reason` is one of the STYLE_REASON
 *  codes; the message carries the context. */
export class SituationStyleError extends Error {
  constructor(reason, message) {
    super(message);
    this.name = "SituationStyleError";
    this.reason = reason;
  }
}

export const STYLE_REASON = Object.freeze({
  INVALID_TEMPLATE: "invalid-template",
  INVALID_TILEJSON: "invalid-tilejson",
});

/** Derives the closest zoom the map allows from the terrain manifest,
 *  mirroring the Apple client's maximumZoomLevel: an unreadable manifest
 *  yields 14; otherwise the deepest band plus OVERZOOM_STEPS. */
export function deriveMaximumZoom(manifest) {
  if (typeof manifest !== "object" || manifest === null) return 14;
  const bands = manifest.bands;
  if (!Array.isArray(bands)) return 14;
  const depths = bands
    .map((band) => band?.max_zoom)
    .filter((depth) => typeof depth === "number" && Number.isFinite(depth));
  const deepest = depths.length > 0 ? Math.max(...depths) : 13;
  return deepest + OVERZOOM_STEPS;
}

/** Converts one exported TileJSON document into an inline tile list for a
 *  style source. Inline tile URLs avoid renderer-specific rules about how a
 *  TileJSON's own relative URLs resolve. */
function inlineTileSource(source, tileJson, assetsBase) {
  const tiles = tileJson?.tiles;
  if (
    !Array.isArray(tiles) ||
    tiles.length === 0 ||
    !tiles.every((entry) => typeof entry === "string") ||
    !Number.isInteger(tileJson.minzoom) ||
    !Number.isInteger(tileJson.maxzoom)
  ) {
    throw new SituationStyleError(
      STYLE_REASON.INVALID_TILEJSON,
      `tile document for ${source} is not a usable TileJSON`,
    );
  }
  return {
    tiles: tiles.map((template) => assetsBase + template),
    minzoom: tileJson.minzoom,
    maxzoom: tileJson.maxzoom,
  };
}

/**
 * Resolves the shared style template against the exported assets.
 *
 * `template` is the parsed SituationStyle.json document. `assetsBase` is the
 * URL prefix the exported asset tree is served under, ending in "/".
 * `coastlineTileJson` and `terrainTileJson` are the parsed exported TileJSON
 * documents. `glyphsPath` is the fonts directory relative to `assetsBase`,
 * or null when the export carries no fonts.
 *
 * Returns a new style object; the template is not modified. Throws
 * SituationStyleError when the template does not carry the expected tokens.
 */
export function resolveSituationStyle({
  template,
  assetsBase,
  coastlineTileJson,
  terrainTileJson,
  glyphsPath,
}) {
  const style = structuredClone(template);
  const sources = style?.sources;
  const coastline = sources?.["pilotage-coastline"];
  const terrain = sources?.["pilotage-terrain"];
  if (coastline?.url !== COASTLINE_TOKEN || terrain?.url !== TERRAIN_TOKEN) {
    throw new SituationStyleError(
      STYLE_REASON.INVALID_TEMPLATE,
      "the style template does not carry the archive URL tokens",
    );
  }
  delete coastline.url;
  Object.assign(
    coastline,
    inlineTileSource("pilotage-coastline", coastlineTileJson, assetsBase),
  );
  delete terrain.url;
  Object.assign(
    terrain,
    inlineTileSource("pilotage-terrain", terrainTileJson, assetsBase),
  );
  // Without a glyph source the renderer draws no text at all. A missing font
  // set costs the labels, not the map: the glyphs key is dropped rather than
  // the style refused.
  if (style.glyphs === `${GLYPHS_TOKEN}/{fontstack}/{range}.pbf`) {
    if (typeof glyphsPath === "string" && glyphsPath.length > 0) {
      style.glyphs = `${assetsBase}${glyphsPath}/{fontstack}/{range}.pbf`;
    } else {
      delete style.glyphs;
    }
  }
  return style;
}

/** The notice each source in the style asks to be shown, read from the style
 *  document so a source added without its notice cannot appear silently.
 *  Mirrors the Apple client's attributions(bundle:). */
export function sourceAttributions(style) {
  const sources = style?.sources;
  if (typeof sources !== "object" || sources === null) return [];
  return Object.values(sources)
    .map((source) => source?.attribution)
    .filter((attribution) => typeof attribution === "string")
    .map((attribution) => attribution.trim())
    .filter((attribution) => attribution.length > 0)
    .sort();
}
