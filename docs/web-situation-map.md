# Web situation map

The web client shows the situation map as a main-view stage. The stage uses
MapLibre GL JS. The Apple client uses MapLibre Native. One style file drives
both renderers: `clients/apple/Resources/SituationStyle.json`. The style
declares the globe projection, and the web renderer draws the globe by
default.

## One style, two renderers

The style document carries three `__PILOTAGE_*__` URL tokens. Each client
substitutes the tokens at run time:

- The Apple client points the tokens at the bundled MBTiles archives and the
  bundled fonts (`SituationStyleResource.swift`).
- The web client points the tokens at the exported asset tree
  (`clients/web/situation-style.js`).

The web client never edits or forks the style. The export script copies the
style verbatim. `scripts/check-web-situation-map.sh` rejects a committed
style copy under `clients/web`.

The camera opens at the same position on both clients: latitude 40.5,
longitude −76.5, zoom 6, pitch 55°, minimum zoom 0. The closest zoom derives
from `SituationTerrain.manifest.json`: the deepest band plus two overzoom
steps. `clients/web/situation-style.test.mjs` reads the Apple sources and
fails when one side changes a value without the other.

## Build steps

MapLibre GL JS cannot read an MBTiles archive, and the web client loads no
external resource at run time. Two scripts prepare the local files. Both
outputs are build artifacts and are not committed.

1. `scripts/vendor-maplibre-web.sh` downloads one pinned MapLibre GL JS
   release archive, verifies its SHA-256 digest, and copies the four runtime
   files into `clients/web/vendor/maplibre-gl/`.
2. `scripts/build-web-situation-assets.sh` exports each MBTiles archive into
   a static `z/x/y` tile tree under `clients/web/situation-assets/`, writes
   one TileJSON document per source, and copies the style, the terrain
   manifest, and the glyph fonts beside them. Build the archives first with
   `clients/apple/scripts/build-situation-coastline.sh` and
   `clients/apple/scripts/build-situation-terrain.sh`.

The export flips each MBTiles row index, because MBTiles counts tile rows
from the south and the web tile URL scheme counts from the north. The export
also decompresses each vector tile, because a static file server sends no
`Content-Encoding` header.

Serve the repository root statically and open the viewer:

    python3 -m http.server 8000
    open http://localhost:8000/clients/web/index.html

Select "situation / globe" in the main-view control.

## Availability

The stage loads the renderer and the assets on the first selection, never
during viewer boot. When a part is missing, the stage shows a typed reason
(ADR-0037) instead of an empty map:

- `MAP_ASSETS_MISSING` — run `scripts/build-web-situation-assets.sh`.
- `MAP_LIBRARY_MISSING` — run `scripts/vendor-maplibre-web.sh`.
- `MAP_STYLE_INVALID` — the style template or the export is not usable.
- `MAP_RENDER_FAILED` — the browser refused the renderer, for example
  without WebGL2.

## Guards

- `scripts/check-web-situation-map.sh` — the boundary: no style fork, no
  committed build artifact, a pinned renderer digest, a dynamic renderer
  import, no run-time network URL, a derived closest zoom, and the typed
  unavailable states.
- `scripts/test-check-web-situation-map.sh` — proves the guard rejects each
  loss.
- `clients/web/situation-map.browser.test.mjs` — a real Chromium selects the
  stage and must see the globe, the shared camera, and the source notices;
  without assets or renderer it must see the typed reason.
