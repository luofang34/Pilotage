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
import {
  NORTH_UP_LABEL,
  headingControlLabel,
  situationCamera,
} from "./situation-camera.js";

/** Camera the map opens with. The values match the Apple client's
 *  SituationMap defaults (situation-style.test.mjs reads the Swift source
 *  and holds the two in step); the closest zoom derives from the terrain
 *  manifest. */
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

/** How many distinct renderer failure classes are logged before the rest
 *  are dropped. Failure messages carry tile URLs, so the class key strips
 *  them; without the cap a misconfigured server would bury every other log
 *  entry with one line per tile. */
const MAX_LOGGED_RENDER_ERRORS = 20;

/** How long the renderer may take to reach its load event before the stage
 *  declares failure. A renderer whose worker died never errors and never
 *  loads; without a deadline the stage would show "loading" forever. */
const LOAD_DEADLINE_MS = 60_000;

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

/** Reports where the camera points, for a reader's own tool and for the
 *  tests that hold the two clients to the same rule. The label is written
 *  only when the heading changes: the move event fires every frame of a
 *  drag, and an accessible name rewritten at that rate is announced at
 *  that rate. */
function watchCamera(map, surface) {
  let last = null;
  const update = () => {
    const camera = situationCamera({
      bearingDeg: map.getBearing(),
      pitchDeg: map.getPitch(),
    });
    surface.dataset.cameraHeading = camera.headingDegrees.toFixed(1);
    surface.dataset.cameraPitch = camera.pitchDegrees.toFixed(1);
    const label = camera.isRotated
      ? headingControlLabel(camera.headingDegrees)
      : NORTH_UP_LABEL;
    if (label !== last) {
      last = label;
      surface.setAttribute("aria-label", label);
    }
  };
  map.on("move", update);
  update();
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

  // Every boot failure ends in a typed state (ADR-0037): an unexpected
  // rejection — a fetch cut off mid-flight, a renderer construct throw —
  // must not strand the stage on the loading notice.
  const boot = async () => {
    try {
      return await bootStage();
    } catch (error) {
      setUnavailable(MAP_REASON.RENDER_FAILED, String(error));
      return null;
    }
  };

  const bootStage = async () => {
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

    // A pointer has no two-finger tilt or turn, so the camera needs a
    // control to reach them: the compass turns and tilts the map when
    // dragged, and faces north and looks straight down again when
    // clicked. Touch reaches the same camera through the renderer's own
    // two-finger handlers, which is why the Apple client can hide its
    // controls until there is something to undo and this one cannot.
    map.addControl(new maplibre.NavigationControl({ visualizePitch: true }), "top-right");
    watchCamera(map, surface);

    const loggedErrors = new Set();
    let suppressionAnnounced = false;
    map.on("error", (event) => {
      const message = String(event?.error ?? "unknown renderer error");
      // Tile URLs make each failure unique; the class key strips them so
      // one failure class logs once.
      const failureClass = message.replace(/\bhttps?:[^\s]*|\/[^\s]*/g, "<url>");
      if (loggedErrors.has(failureClass)) return;
      if (loggedErrors.size >= MAX_LOGGED_RENDER_ERRORS) {
        if (!suppressionAnnounced) {
          suppressionAnnounced = true;
          log("situation map: further renderer errors suppressed");
        }
        return;
      }
      loggedErrors.add(failureClass);
      log(`situation map: ${message}`);
    });

    return new Promise((resolve) => {
      const deadline = setTimeout(() => {
        map.remove();
        setUnavailable(
          MAP_REASON.RENDER_FAILED,
          `the renderer did not reach load within ${LOAD_DEADLINE_MS / 1000} s`,
        );
        resolve(null);
      }, LOAD_DEADLINE_MS);
      map.once("load", () => {
        clearTimeout(deadline);
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
