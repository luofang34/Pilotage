# ADR-0038: One surface model for operator screens

- Status: Proposed (design only; implementation deferred)
- Date: 2026-08-17
- Depends on: [ADR-0037](0037-modular-operator-client-composition.md)

## Context

The iPad client grew three controls that all change what fills the screen:

- A map toggle in the rack header. It switches between "rack beside map"
  and "rack alone".
- A focus toggle on each tile. It gives one tile the whole rack column.
- An instrument-stack switch in the settings. It enables the rack at all.

Each control answers the same question — "what is on screen" — at a
different layer, with a different word. Operators cannot predict what a
press does. The controls also fight for space: the bar under the rack
covered instruments until this week.

Apple platforms already have one vocabulary for this. Mail on iPadOS uses
`NavigationSplitView` with two collapsible levels: the user shows or
hides columns with one affordance, and the content of each column is a
selection, not a mode. Photos promotes a grid item to one-up through
navigation, not through a zoom switch.

visionOS makes the question sharper. There, the USER owns the layout:
windows and volumes go where the operator puts them. An application that
hardcodes "rack is a VStack beside a map" cannot exist on that platform.
An application that can present each of its surfaces on its own adapts
without redesign. visionOS also provides ornaments: controls that attach
to a window's edge and never cover its content — the permanent answer to
the occlusion class of bugs.

## Decision (proposed)

### The model

Separate two questions that the current controls mix:

1. **Which regions are visible.** A region is a place content can go:
   the primary surface, the instrument column, and (future) secondary
   windows. Region visibility belongs to the platform's own navigation
   affordance — `NavigationSplitView` column state on iPadOS, window
   existence on visionOS. No custom toggles.
2. **What content each region shows.** Content units are *surfaces*:
   the map, one video source, one instrument panel. Assignment is a
   selection ("show this here", "swap with primary"), persisted per
   size class. Promotion of a tile to a larger region replaces the
   focus/enlarge buttons.

One state object, `SurfaceLayout`, owns both answers. Views render it;
no view owns layout policy.

### iPadOS mapping

- `NavigationSplitView`: instrument column as the supplementary column,
  primary surface (map, or a promoted surface) as detail.
- The system sidebar button replaces the map toggle. Column width is the
  system's drag affordance, not a computed constant.
- Tap a tile: select it. Selection promotes within the column (today's
  focus). "Swap with primary" moves it to the detail region — video
  becomes the flying surface, map goes to the column, one gesture back.
- Compact width degrades through the same `NavigationSplitView` to a
  stack, for free.
- The settings instrument-stack switch is deleted. Column visibility IS
  that switch.

### visionOS mapping

- Each surface is independently presentable: one `WindowGroup` per
  surface kind, parameterized by surface id. The operator places panels,
  video, and map as separate windows; `SurfaceLayout` records what is
  open, the system records where.
- The control bar (telegraph, release, connection chip) becomes an
  ornament on the primary window. It can never cover an instrument.
- The frame path already fits: `VideoFrameHub` paints one `CALayer` per
  attached surface, wherever that layer's window lives.
- Control input is unchanged: the GameController framework and the
  shared control runtime are platform-independent.

### What this deletes

- The map toggle, the per-tile focus button, and the instrument-stack
  setting — replaced by column state plus surface selection.
- The `idealWidth` chrome arithmetic — the platform owns column width.
- The full-screen video cover (already removed) stays removed; promotion
  to the primary region is the one enlargement.

## Migration (all deferred)

1. Introduce `SurfaceLayout` behind the current views. No visible change.
2. Adopt `NavigationSplitView` on iPadOS: column = rack, detail = map.
3. Replace focus with selection and add "swap with primary".
4. Add the visionOS target: per-surface `WindowGroup`s, telegraph as an
   ornament.

## Consequences

- One vocabulary for layout on every platform; the three-switch muddle
  and its occlusion bugs cannot come back.
- Surfaces become addressable units. Future needs — two videos at once,
  picture-in-picture, an operator-defined column — are assignments, not
  redesigns.
- The web client keeps its own layout idiom; this record binds only the
  Apple ports. Shared control and instrument cores are unaffected.
