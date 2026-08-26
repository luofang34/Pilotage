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
import { attachOwnship } from "./situation-ownship.js";
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
  centerLatitudeDeg: 47.4,
  centerLongitudeDeg: 8.55,
  zoomLevel: 9,
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
/** How often the mark's own clock ticks. Short enough that a reader never
 *  meets a mark much past its 3 s window, long enough to be free. */
const OWNSHIP_AGE_INTERVAL_MS = 500;

export function wireSituationMapStage(doc, { log = () => {} } = {}) {
  const win = doc.defaultView ?? globalThis;
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

  // The vehicle mark, once the renderer exists. Until then a sample has
  // nowhere to draw and is dropped rather than queued: a position is only
  // worth showing while it is current.
  let ownship = null;
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
      // A throw after the mark was bound leaves it live and updating on a
      // stage that reports itself unavailable.
      ownship?.age(Number.POSITIVE_INFINITY);
      ownship = null;
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

    const sourceNames = Object.keys(manifest.sources ?? {});
    const [template, terrainManifest, ...tileJsonDocuments] = await Promise.all([
      fetchJson(ASSETS_BASE + manifest.style),
      fetchJson(ASSETS_BASE + manifest.terrain_manifest),
      ...sourceNames.map((name) => fetchJson(ASSETS_BASE + manifest.sources[name])),
    ]);
    const tileJsonBySource = Object.fromEntries(
      sourceNames.map((name, index) => [name, tileJsonDocuments[index]]),
    );
    if (template === null || tileJsonDocuments.some((document) => document === null)) {
      setUnavailable(MAP_REASON.STYLE_INVALID, "the asset export is incomplete");
      return null;
    }

    let style;
    try {
      style = resolveSituationStyle({
        template,
        assetsBase: ASSETS_BASE,
        tileJsonBySource,
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
    ownship = attachOwnship(maplibre, map, surface);

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
        // The mark belongs to a map that no longer exists. Withdrawing it
        // first clears what it already wrote: a stage that reports itself
        // unavailable beside a surface that reports a vehicle shown, at a
        // position, is two answers to one question. Releasing the handle
        // after that stops the next sample from writing them again.
        ownship?.age(Number.POSITIVE_INFINITY);
        ownship = null;
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

  // A link that goes silent delivers no sample to notice the silence with,
  // so the mark is aged on a clock of its own. Without this the vehicle
  // stays on the map at its last position for as long as the page is open,
  // which is the one thing the mark must never do. The stage owns the
  // clock because the stage owns the mark.
  const ageOwnship = () => ownship?.age(performance.now());
  let ownshipAging = setInterval(ageOwnship, OWNSHIP_AGE_INTERVAL_MS);
  // `pagehide` fires for the back/forward cache as well as for a real
  // unload, and a page restored from that cache resumes its telemetry. A
  // clock stopped without re-arming would leave the mark updating and
  // never ageing for the rest of the page's life.
  win.addEventListener("pagehide", () => {
    clearInterval(ownshipAging);
    ownshipAging = null;
  });
  win.addEventListener("pageshow", () => {
    if (ownshipAging === null) ownshipAging = setInterval(ageOwnship, OWNSHIP_AGE_INTERVAL_MS);
    ageOwnship();
  });
  // A hidden tab clamps its timers, so the mark may be a minute stale by
  // the time a reader looks at it again. Age it before they do.
  doc.addEventListener("visibilitychange", () => {
    if (doc.visibilityState === "visible") ageOwnship();
  });

  return {
    activate,
    booted: () => bootPromise,
    /** Takes one telemetry sample for the vehicle mark. */
    observeTelemetry: (telemetry, nowMs) => ownship?.observe(telemetry, nowMs),
    /** Withdraws a mark whose fix stopped arriving, on the stage's own
     *  clock. Exposed so a test can drive it deterministically. */
    ageOwnship: (nowMs) => ownship?.age(nowMs ?? performance.now()),
  };
}
