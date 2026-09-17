# Mission planner and navigation search

## Implemented scope

The Apple app has a local mission planner.
The planner uses `pilotage-planning` through the Swift binding.
The planner saves one working mission in Application Support.
It writes each change through a serial file store.
A failed read does not replace the saved file with an empty mission.

The planner supports these operations:

- Search installed airports, navigation aids, and waypoints.
- Search a published identifier, name, or region.
- Select one result when an identifier has several matches.
- Enter waypoint identifiers separated by spaces.
- Drag waypoint chips to change route order.
- Add a waypoint with WGS 84 decimal coordinates.
- Replace, remove, and reorder route waypoints.
- Calculate great-circle distance and initial track in degrees true.
- Select, create, edit, import, and export vehicle profiles.
- Calculate elapsed time from the selected cruise profile.
- Calculate arrival time from an entered departure time.
- Check source validity at each calculated arrival.
- Show planned routes on the existing globe.
- Save separate vehicle assignments in one mission.
- Assign responsibility to a person with an optional organization name.
- Group assignments into a swarm.
- Set individual and swarm times on target.
- Export the mission and a content digest for review.

The app uses the native SwiftUI search interface.
Apple describes this interface in
[Adding a search interface to your app](https://developer.apple.com/documentation/swiftui/adding-a-search-interface-to-your-app).
The map controls retain native Liquid Glass.
Selecting another assignment does not reset the map camera or renderer.
The main editor shows one route, its vehicle profile, and departure time.
The assignment selector appears when the mission has more than one vehicle.
The Team & Timing button opens the coordination controls.
Route waypoints use a compact layout with text entry and drag reordering.
The text field shares a line with the waypoint chips.
Tap a chip to edit its identifier in place.
Backspace in an empty field removes the preceding waypoint and retains keyboard focus.
Backspace in an empty waypoint edit removes that waypoint.
The route action menu supports insertion before or after the edited waypoint.
The menu also supports copy and undo.
The text editor accepts waypoint identifiers and `DCT` separators.
It accepts ASCII letters, digits, and spaces.
It converts lowercase letters to uppercase letters.
It rejects other characters in typed or pasted text.
It does not expand airways or procedure names.
Return resolves the complete entry before the route changes.
The keyboard remains ready for the next entry.
The text input retains additional characters entered during a navigation lookup.
The route can also open as a native text view for multiple-identifier edits.
An indirect Scribble interaction provides a separate target for each badge and the entry field.
Writing on a badge edits that waypoint at the Pencil position.
Writing in the entry field keeps its route insertion point.
The route action menu opens the complete route as editable text.
The text view supports native text selection and Apple Pencil editing gestures.
On first launch, the app opens navigation data before it imports the chart packages.
A submitted entry waits for navigation data to open.
Editing or cancelling the entry cancels that pending submission.
An unresolved identifier leaves the installed route unchanged.
The editor shows matching points beside the route.
It orders returned matches by distance from the edited waypoint or the insertion point.
An exact identifier takes precedence over a prefix match.
The app does not select an ambiguous location automatically.
The distance button shows the route on the map.
See the [route editor workflows](route-editor-workflows.md) for the flight and amendment steps.

## Navigation storage

A source edition is an immutable set of navigation records.
Its identity contains the release ID, authority, edition, input digest, and valid period.
The valid period uses UTC Unix seconds.
The start is included. The end is excluded.

`NavigationDataset` is the input contract for a source converter.
The contract accepts any authority name.
It does not require an FAA authority enum.
Each point has a source record key, identifier, type, name, region, and WGS 84 position.
The key must be unique within the source edition.

`NavigationIndex` stores one edition in SQLite.
SQLite is an embedded database.
The index uses the Full-Text Search 5 extension for identifier and name queries.
It treats search words as literal prefixes.
It does not interpret user text as query commands.
The index keeps separate records for duplicate identifiers.

The `nav_sqlite` artifact has database schema version 1.
It contains these tables:

| Table | Content |
|---|---|
| `navigation_metadata` | The source identity as JSON. |
| `points` | Source-qualified navigation points. |
| `points_search` | The derived full-text index. |

The package store verifies artifact sizes and SHA-256 digests before use.
The search loader also checks the database schema and release identity.
It replaces the open source set only when all new sources open successfully.

An ACNAV artifact is an encoded AeroContext navigation snapshot.
The app can build a local search index from an installed ACNAV artifact.
The cache name includes the source digest and release identity.
ACNAV stores effective dates. The verified release supplies the exact UTC instants.

Search selects one installed edition per authority and coverage area.
The newest current edition takes precedence over an expired selection.
Search uses the newest expired edition when no current edition is installed.
An upcoming edition does not replace the fallback before its effective time.
The search view shows each included source and its validity.
Search combines NASR and CIFP records for the same point.
Records must have the same identifier and type.
Their positions must be within 0.1 nautical mile.
Current records take precedence over expired records.
Equal identifiers at different positions remain separate results.

Each route waypoint stores the selected point and its source identity.
A new data edition does not change these coordinates.
The app retains the source packages used by the working mission.
Removing a waypoint can release a package when no mission waypoint uses it.
The app removes a superseded edition after its current replacement opens successfully.
The app also removes the derived search cache for that edition.
A selection, saved mission, or dependent package can retain an older edition.
Retention does not put that edition back into current search results.

## Vehicle profiles

A vehicle profile contains its name, class, identifier, cruise speed, and performance source.
Aircraft cruise speed uses true airspeed.
True airspeed is speed relative to the air mass.
Aircraft route time assumes still air.
Surface vehicle profiles use ground speed.
The profile editor permits an explicit speed-reference selection.
These profiles do not calculate wind, climb, descent, or fuel.

The Vehicles sheet has New, Import, and Export controls.
The library is stored in `Missions/vehicles.json`.
Import and export use `VehicleProfileDocument` JSON with schema version 1.
The [profile example](examples/vehicle-profiles.json) shows the file format.
Replace its example values with verified vehicle data before use.
The app validates the complete import before it changes the library.
A conflicting profile ID causes the import to fail.
The app does not replace an unreadable library with an empty library.

Each route retains a copy of the selected profile and its revision.
A library edit does not change an existing route estimate.
Select the updated profile to apply it to a route.
A ground-speed override in an existing draft remains visible in Details.
Selecting a profile clears that override.
An adapter for a ForeFlight aircraft export is not included.

## Route display and local navigation

All route legs are blue during planning.
Navigate on this iPad starts local route progress.
It requests a device position if no position source is available.
The current leg is magenta. Past legs are orange.
These colors follow the
[ForeFlight route legend](https://cloudfront.foreflight.com/docs/ff/14.6/Foreflight%20Legends%20Guide%20v14.6.pdf).
Other vehicle routes use lower opacity.
Direct legs remain solid.
The NavLog also identifies each leg with a text label.

The reader can set a current leg from a waypoint menu.
Mark reached advances to the next waypoint.
A position within 0.1 nautical mile of the current waypoint also advances the route.
Only the locally selected navigation assignment receives this position update.
An edit during local navigation opens a route update review.
The current route stays active during this review.
The review shows the current and proposed waypoint sequences.
The reader selects the waypoint toward which navigation will continue.
Cancel keeps the current route.
Apply update changes the route and the selected navigation leg together.
The app rejects a proposal if its source route changed during review.
An unreviewed model change stops local navigation.
Local navigation does not send vehicle commands.

The renderer clips projected legs at the view boundary.
A leg can cross the screen when both endpoints are outside the screen.
The renderer keeps separate segments at the antimeridian and the globe horizon.

## Local FAA conversion

The converter accepts a NASR CSV ZIP archive or an extracted `FAACIFP18` file.
It uses the AeroContext source parsers.
It checks the source effective date against the requested cycle.
It writes ACNAV, a search database, a release manifest, and an ingest report.
The output directory must not exist.

```sh
cargo run -p pilotage-planning --features ingest \
  --example prepare_navigation_release -- \
  nasr /path/to/03_Sep_2026_CSV.zip /path/to/new-release \
  faa-nasr-2609-r1 2026-09-03T09:01:00Z 2026-10-01T09:01:00Z

cargo run -p pilotage-data-packages --example install_local -- \
  /path/to/new-release/release.json /path/to/new-release /path/to/store
```

Use `cifp` in place of `nasr` for an extracted CIFP file.
Read the ingest report before release publication.
The converter marks its output as development data.
It does not publish a catalog or assert distribution permission.

CIFP contains more than searchable points.
The [FAA CIFP description](https://www.faa.gov/air_traffic/flight_info/aeronav/digital_products/cifp/)
states that it uses raw ARINC data and has a 28-day update interval.
The point converter does not create executable procedure legs.
Procedure charts and executable procedure data use separate contracts.
The planner opens installed procedure charts through Procedure charts.
It lists airports on the route first.
It uses the current release for each authority and coverage area.
It keeps an expired release as a fallback until an effective replacement is installed.
The chart reader shows the edition and valid period.
It opens the verified PDF in a page presentation.
The PDF does not add executable approach or missed-approach legs to the route.

## Other source authorities

A source adapter must convert its input into `NavigationDataset` JSON.
The adapter must preserve the source authority, edition, region, and record keys.
The adapter must supply verified WGS 84 positions and the exact valid period.
The adapter must not assign an FAA authority to foreign records.

```sh
cargo run -p pilotage-planning --example build_navigation_index -- \
  json /path/to/normalized-source.json /path/to/navigation.sqlite source-release-id
```

Package the database as a `nav_sqlite` artifact in a navigation release.
Use the existing package catalog, download, verification, and selection process.
Retain the original source files and conversion report at the publisher.
The app needs the normalized artifacts for offline use.
An adapter for a specific foreign source format is not included.

## Coordinated timing

Time on target, or ToT, is the required arrival time at a selected route occurrence.
A route occurrence identifies one visit to a waypoint.
A repeated visit to the same airport has a different occurrence ID.

An individual ToT names one assignment and one route occurrence.
A swarm ToT names all assignments in that swarm.
Each swarm member selects its own route occurrence.
Each member can have a time offset from the common ToT.
A positive offset puts the member after the common time.
The tolerance permits the same deviation before and after the member time.

The planner calculates the departure window from distance and profile cruise speed.
An explicit ground-speed override takes precedence when one is present.
It intersects the windows for all targets on an assignment.
It reports a conflict when those windows do not intersect.
It reports `Unknown` when arrival time cannot be calculated.
It does not treat an unknown arrival as on time.
Changing swarm membership or deleting a target waypoint cannot silently change a ToT.
Remove the affected timing condition before that change.

The time estimate uses the entered cruise profile.
It does not include wind, climb, descent, holds, or procedure restrictions.
A valid estimate does not prove that a vehicle can execute the plan.

## Review and execution boundary

The exported review document contains the resolved plan, timing assessment, and content digest.
The digest changes when route, ownership, or timing content changes.
The document contains local participant labels.
These labels are not authenticated identities or owner acceptance records.

The implementation does not send mission commands to a host.
It does not provide online editing, owner acceptance, or synchronized vehicle release.
Those functions use the authority and revision rules in the
[coordinated mission proposal](../coordinated-missions-proposal.md).
The host must accept an immutable resolved revision before execution.
It must not resolve this draft again from an unqualified route string.
