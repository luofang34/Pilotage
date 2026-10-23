// Resolve the shared Apple/web style's archive tokens to exported URLs.
// Invalid templates fail explicitly; missing fonts remove labels only.
// Zoom limits come from the terrain manifest, as in SituationStyleResource.

export const GLYPHS_TOKEN = "__PILOTAGE_GLYPHS_URL__";
export const COASTLINE_TOKEN = "__PILOTAGE_COASTLINE_MBTILES_URL__";
export const TERRAIN_TOKEN = "__PILOTAGE_TERRAIN_MBTILES_URL__";

/** Several sources can read one archive at different zoom ranges.
 *  Resolve their tokens so every source uses its exported archive. */
const ARCHIVE_EXPORTS = Object.freeze({
  [COASTLINE_TOKEN]: "pilotage-coastline",
  [TERRAIN_TOKEN]: "pilotage-terrain",
});

/** Limit stretching of the deepest terrain tile. Match the Apple client's
 *  overzoomSteps so both renderers stop at the same closest zoom. */
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
  // The Swift resolver casts bands to a list of dictionaries as a whole;
  // one entry of another shape makes the manifest unreadable.
  if (
    !Array.isArray(bands) ||
    bands.some((band) => typeof band !== "object" || band === null || Array.isArray(band))
  ) {
    return 14;
  }
  const depths = bands
    .map((band) => band.max_zoom)
    .filter((depth) => typeof depth === "number" && Number.isFinite(depth));
  const deepest = depths.length > 0 ? Math.max(...depths) : 13;
  return deepest + OVERZOOM_STEPS;
}

/** Inline URLs avoid renderer-specific relative URL resolution. Preserve
 *  each source's zoom cap so global coverage does not request sparse
 *  deeper tiles. The declared range must stay within the archive's. */
function inlineTileSource(source, declared, tileJson, assetsBase) {
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
    minzoom: Math.max(declared.minzoom ?? tileJson.minzoom, tileJson.minzoom),
    maxzoom: Math.min(declared.maxzoom ?? tileJson.maxzoom, tileJson.maxzoom),
  };
}

/**
 * Resolves the shared style template against the exported assets.
 *
 * `template` is the parsed SituationStyle.json document. `assetsBase` is the
 * URL prefix the exported asset tree is served under, ending in "/".
 * `tileJsonBySource` maps each style source name to its parsed exported
 * TileJSON document. `glyphsPath` is the fonts directory relative to
 * `assetsBase`, or null when the export carries no fonts.
 *
 * Returns a new style object; the template is not modified. Throws
 * SituationStyleError when the template does not carry the expected tokens.
 */
export function resolveSituationStyle({
  template,
  assetsBase,
  tileJsonBySource,
  glyphsPath,
}) {
  const style = structuredClone(template);
  const sources = style?.sources;
  const resolvedTokens = new Set();
  for (const [name, source] of Object.entries(sources ?? {})) {
    const token = source?.url;
    if (typeof token !== "string" || !/^__PILOTAGE_.*__$/.test(token)) continue;
    const archive = ARCHIVE_EXPORTS[token];
    if (archive === undefined) {
      throw new SituationStyleError(
        STYLE_REASON.INVALID_TEMPLATE,
        `the style source ${name} carries ${token}, which names no exported archive`,
      );
    }
    delete source.url;
    Object.assign(
      source,
      inlineTileSource(name, source, tileJsonBySource?.[archive], assetsBase),
    );
    resolvedTokens.add(token);
  }
  for (const token of Object.keys(ARCHIVE_EXPORTS)) {
    if (!resolvedTokens.has(token)) {
      throw new SituationStyleError(
        STYLE_REASON.INVALID_TEMPLATE,
        `the style template does not carry the archive URL token ${token}`,
      );
    }
  }
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
 *  Sources reading one archive carry one notice, listed once. Mirrors the
 *  Apple client's attributions(bundle:). */
export function sourceAttributions(style) {
  const sources = style?.sources;
  if (typeof sources !== "object" || sources === null) return [];
  return Object.values(sources)
    .map((source) => source?.attribution)
    .filter((attribution) => typeof attribution === "string")
    .map((attribution) => attribution.trim())
    .filter((attribution) => attribution.length > 0)
    .filter((attribution, index, all) => all.indexOf(attribution) === index)
    .sort();
}
