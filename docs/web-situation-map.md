# Web situation map

The web client shows the situation map as a main-view stage. The stage uses
MapLibre GL JS. The Apple client uses MapLibre Native. One style file drives
both renderers: `clients/apple/Resources/SituationStyle.json`. The style
declares the globe projection. The web renderer draws the globe by default.

## One style, two renderers

The style document carries a `__PILOTAGE_*__` URL token for each archive
it reads and for its fonts. Each client substitutes the tokens at run
time:

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

## Palette

The base map reads as an aeronautical chart. The elevation tints follow
the FAA VFR sectional ladder, from the sectional's sea-level green through
cream and tan to brown. The band edges are the sectional's own: 1000,
2000, 3000, 5000, 7000, 9000, and 12 000 ft, converted to metres. The two
water tones are the sectional's "Open Water" and "Inland Water".

We read these values from the legend of the FAA Aeronautical Chart User's
Guide, because the FAA publishes no colour table. Two editions of the
guide agree to within a few steps of 255. The ramp continues above the
sectional's highest band and below sea level, where the sectional's legend
stops; those parts are ours, not the FAA's. Below sea level the ramp
reuses the tint of the 1000 to 2000 ft band, as the sectional does, so a
tint alone does not say which of the two a reader is looking at.

Water draws over the terrain relief and over the hillshade. An elevation
ramp cannot distinguish a lake at 500 m from ground at 500 m, so the
polygon that carries that information keeps its own colour instead of
taking the tint of the height below it.

The hillshade uses the Igor method, which darkens a slope without a stain
on the tint below it. It lights the terrain from the north west, as a
chart does.

## Drainage

The rivers are centre lines, and they stay centre lines at every zoom.
Natural Earth publishes no river areas, so a river has a width only where
a lake polygon gives it one. A sectional chart draws its rivers the same
way, and thins them by rank; the style follows that, and widens a line
with the zoom rather than pretending to a shape the data does not carry.

Three files feed the river layer: the global 1:10m network, and the
regional files for Europe and North America. The lakes layer takes the two
regional files as well.

The rank is a drawing scale, not a name for which file a feature came from.
The global file holds mostly rank 0 to 9 and a few dozen features above it;
the regional files hold rank 10 to 12 only. The ladder therefore reads the
rank and not the file: it draws nothing above rank 9 below zoom 9, and
fades those features in between zoom 9 and zoom 11.

The shape of the coast is generalized at 1:10,000,000. It disagrees with
the terrain relief by some hundred metres. The disagreement is visible when
a reader zooms in far, and it is the same everywhere, because one source
draws the whole world.

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

The two archives hold their zoom bands differently, because the renderer
treats a missing tile differently in each.

A raster tile that the terrain archive does not hold is drawn from the
tile above it, stretched. The terrain archive can therefore hold a world
band plus deeper bands over the area that is flown: the relief outside
those bands is coarse, and it is always drawn.

A vector tile that the coastline archive does not hold draws nothing at
all, and no shallower tile stands in for it. A band that stops at a
longitude therefore stops the land and the sea at a straight line, and the
reader sees a rectangle of bare background. The coastline archive is for
that reason complete over the world at every zoom it holds. Above its
deepest zoom the renderer stretches the tiles that it has, so the picture
changes with the zoom and never with the position of the reader.

The cost of that completeness sets the depth. The archive stops at zoom 7:
each deeper zoom multiplies the tile count and the size by four, and the
1:10m source data does not carry more shape than zoom 7 shows. The
manifest records what the build produced.

Serve the repository root statically and open the viewer:

    python3 -m http.server 8000
    open http://localhost:8000/clients/web/index.html

Select "situation / globe" in the main-view control.

## Camera controls

Touch reaches the camera with two fingers: two fingers that move together
tilt the map, and two fingers that turn it rotate the map. A pointer has
neither gesture, so the map carries a compass. Drag the compass to turn
and tilt the map. Click it to face north and look straight down again. A
drag on the map with the control key held turns and tilts it too.

The Apple client hides its controls until there is something to undo,
because touch reaches the camera without them. A pointer reaches the
camera only through the compass, so the compass is always on the map.

`clients/web/situation-camera.js` holds the thresholds and the wording the
two clients share, and `situation-camera.test.mjs` reads the Apple sources
so a value changed on one client fails until the other follows.

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
