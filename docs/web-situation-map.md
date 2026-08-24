# Web situation map

The web client shows the situation map as a main-view stage. The stage uses
MapLibre GL JS. The Apple client uses MapLibre Native. One style file drives
both renderers: `clients/apple/Resources/SituationStyle.json`. The style
declares the globe projection. The web renderer draws the globe by default.

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

MapLibre GL JS cannot read an MBTiles archive. The web client loads no
external resource at run time. Two scripts prepare the local files. Both
outputs are build artifacts. Do not commit them.

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

The archives hold zoom bands: a world band plus deeper regional bands. A
view outside a deep band shows no coastline fill at that zoom, and the
terrain stretches its deepest world tile. Both renderers show a banded
archive the same way.

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
- `MAP_RENDER_FAILED` — the browser refuses the renderer, for example
  without WebGL2, or the boot fails in an unexpected way.

## Guards

`scripts/check-web-situation-map.sh` holds the boundary. It rejects a style
fork under `clients/web`, a committed build artifact, an unpinned renderer
digest, a static renderer import, a network URL in the situation modules, a
hand-written closest zoom, a lost typed state, and a CI workflow that no
longer runs these guards. `scripts/test-check-web-situation-map.sh` proves
that the guard rejects each loss.

`clients/web/situation-map.browser.test.mjs` boots the stage in a real
Chromium. With exported assets the test must see the globe, the shared
camera, and the source notices. Without the assets or without the renderer
the test must see the typed reason.
