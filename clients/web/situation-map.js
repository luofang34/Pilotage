// Situation map stage (MapLibre GL JS) for the web client.
//
// The map is a situation module surface (ADR-0037): read-only, never a
// placeholder. The stage stays inert until the reader selects it; the first
// selection loads the vendored renderer and the exported assets. When either
// is missing the stage reports a typed reason instead of an empty map.
//
// The renderer import stays dynamic: the vendor directory is a build
// artifact, and viewer boot must complete without it.

import {
  deriveMaximumZoom,
  resolveSituationStyle,
  SituationStyleError,
} from "./situation-style.js";

/** Camera the map opens with. The values match the Apple client's
 *  SituationMap defaults (scripts/check-web-situation-map.sh holds the two
 *  in step); the closest zoom derives from the terrain manifest. */
export const INITIAL_CAMERA = Object.freeze({
  centerLatitudeDeg: 40.5,
  centerLongitudeDeg: -76.5,
  zoomLevel: 6,
  pitchDegrees: 55,
  minimumZoomLevel: 0,
});

export const MAP_REASON = Object.freeze({
  LIBRARY_MISSING: "MAP_LIBRARY_MISSING",
  ASSETS_MISSING: "MAP_ASSETS_MISSING",
  STYLE_INVALID: "MAP_STYLE_INVALID",
  RENDER_FAILED: "MAP_RENDER_FAILED",
});

const VENDOR_MODULE = "./vendor/maplibre-gl/maplibre-gl.mjs";
const VENDOR_STYLESHEET = new URL("./vendor/maplibre-gl/maplibre-gl.css", import.meta.url);
const ASSETS_BASE = new URL("./situation-assets/", import.meta.url).href;

/** How many distinct renderer error messages are logged before the rest are
 *  dropped. A sparse tile band answers many fetches with 404, and one line
 *  per missing tile would bury every other log entry. */
const MAX_LOGGED_RENDER_ERRORS = 20;

async function fetchJson(url) {
  const response = await fetch(url);
  if (!response.ok) return null;
  return response.json();
}

function loadStylesheet(doc, href) {
  return new Promise((resolve) => {
    const link = doc.createElement("link");
    link.rel = "stylesheet";
    link.href = href;
    link.addEventListener("load", () => resolve(true), { once: true });
    link.addEventListener("error", () => resolve(false), { once: true });
    doc.head.append(link);
  });
}

/**
 * Wires the situation map stage: watches the main-view selector and boots
 * the map on the first selection. Returns the wiring handle with the boot
 * promise for observation; the handle is not needed for operation.
 */
export function wireSituationMapStage(doc, { log = () => {} } = {}) {
  const figure = doc.getElementById("stage-map");
  const surface = doc.getElementById("situationMap");
  const mainView = doc.getElementById("mainView");
  if (!figure || !surface || !mainView) return null;

  const notice = doc.createElement("div");
  notice.className = "map-notice";
  notice.setAttribute("role", "status");
  surface.parentElement.append(notice);

  figure.dataset.mapState = "idle";

  const setUnavailable = (reason, detail) => {
    figure.dataset.mapState = "unavailable";
    figure.dataset.mapReason = reason;
    notice.textContent = `situation map unavailable: ${reason} — ${detail}`;
    log(`situation map unavailable: ${reason} — ${detail}`);
  };

  let bootPromise = null;
  const activate = () => {
    bootPromise ??= boot();
    bootPromise.then((map) => map?.resize()).catch(() => {});
  };

  const boot = async () => {
    figure.dataset.mapState = "loading";
    notice.textContent = "situation map loading…";

    const manifest = await fetchJson(`${ASSETS_BASE}assets-manifest.json`);
    if (manifest === null) {
      setUnavailable(
        MAP_REASON.ASSETS_MISSING,
        "run scripts/build-web-situation-assets.sh",
      );
      return null;
    }

    let maplibre;
    try {
      maplibre = await import(VENDOR_MODULE);
    } catch {
      setUnavailable(
        MAP_REASON.LIBRARY_MISSING,
        "run scripts/vendor-maplibre-web.sh",
      );
      return null;
    }
    await loadStylesheet(doc, VENDOR_STYLESHEET.href);

    const [template, coastlineTileJson, terrainTileJson, terrainManifest] =
      await Promise.all([
        fetchJson(ASSETS_BASE + manifest.style),
        fetchJson(ASSETS_BASE + manifest.sources?.["pilotage-coastline"]),
        fetchJson(ASSETS_BASE + manifest.sources?.["pilotage-terrain"]),
        fetchJson(ASSETS_BASE + manifest.terrain_manifest),
      ]);
    if (template === null || coastlineTileJson === null || terrainTileJson === null) {
      setUnavailable(MAP_REASON.STYLE_INVALID, "the asset export is incomplete");
      return null;
    }

    let style;
    try {
      style = resolveSituationStyle({
        template,
        assetsBase: ASSETS_BASE,
        coastlineTileJson,
        terrainTileJson,
        glyphsPath: typeof manifest.glyphs === "string" ? manifest.glyphs : null,
      });
    } catch (error) {
      if (error instanceof SituationStyleError) {
        setUnavailable(MAP_REASON.STYLE_INVALID, `${error.reason}: ${error.message}`);
        return null;
      }
      throw error;
    }

    let map;
    try {
      map = new maplibre.Map({
        container: surface,
        style,
        center: [INITIAL_CAMERA.centerLongitudeDeg, INITIAL_CAMERA.centerLatitudeDeg],
        zoom: INITIAL_CAMERA.zoomLevel,
        pitch: INITIAL_CAMERA.pitchDegrees,
        minZoom: INITIAL_CAMERA.minimumZoomLevel,
        maxZoom: deriveMaximumZoom(terrainManifest),
      });
    } catch (error) {
      setUnavailable(MAP_REASON.RENDER_FAILED, String(error));
      return null;
    }

    const loggedErrors = new Set();
    map.on("error", (event) => {
      const message = String(event?.error ?? "unknown renderer error");
      if (loggedErrors.size >= MAX_LOGGED_RENDER_ERRORS || loggedErrors.has(message)) {
        return;
      }
      loggedErrors.add(message);
      log(`situation map: ${message}`);
    });

    return new Promise((resolve) => {
      map.once("load", () => {
        figure.dataset.mapState = "ready";
        delete figure.dataset.mapReason;
        // Observability for tests and for a reader checking behaviour
        // parity: what the renderer actually applied.
        figure.dataset.mapProjection = map.getProjection()?.type ?? "";
        figure.dataset.mapZoom = String(map.getZoom());
        figure.dataset.mapPitch = String(map.getPitch());
        figure.dataset.mapMaxZoom = String(map.getMaxZoom());
        const center = map.getCenter();
        figure.dataset.mapCenter = `${center.lat},${center.lng}`;
        notice.remove();
        resolve(map);
      });
    });
  };

  mainView.addEventListener("change", () => {
    if (mainView.value === figure.id) activate();
  });
  // The selector may already point at the map when wiring runs (URL restore,
  // or a change fired before this module finished loading).
  if (mainView.value === figure.id) activate();

  return { activate, booted: () => bootPromise };
}
