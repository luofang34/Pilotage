# Route editor workflows

## Plan one flight

1. Open Route Plan.
2. Select a saved vehicle profile, or create or import a profile.
3. Type route identifiers separated by spaces.
4. Press Return to add the complete entry.
5. Select a result if an identifier has several locations.
6. Select the distance button to show the route on the map.
7. Set Departure if arrival times are required.
8. Use NavLog to inspect distance, track, and elapsed time.
9. Select Navigate on this iPad to start local route progress.

The Team & Timing button opens coordinated planning.
The single-vehicle editor does not require coordination fields.
The time estimate uses the selected vehicle profile.
The Still air label identifies an estimate without wind correction.
Return keeps the keyboard open. Continue with the next identifier without another tap.
Text entered during a navigation lookup remains in the entry field.

## Edit a route

Tap a waypoint chip to edit that identifier.
The text field occupies the chip position.
The route action menu can insert a waypoint before or after this position.
The menu can remove the waypoint or show its position on the map.

Backspace in an empty entry removes the preceding waypoint.
Each additional press removes the next preceding waypoint.
The insertion position remains at the deletion point.
Backspace in an empty waypoint edit removes the waypoint.
Cancel edit restores the displayed chip without changing the route.

Return resolves all identifiers in the entry before it changes the route.
Invalid text or an unresolved point does not apply a partial route.
The editor retains the occurrence ID when it replaces a waypoint.
This ID identifies a specific visit to a waypoint.
An additional visit receives a new ID.

Matching results show distance from the edited waypoint.
An insertion uses the preceding waypoint as its reference.
Search by name remains available through the search button.
Distance orders the returned matches. It is not a nearest-airport query.
The reader selects a location when more than one exact match exists.

Drag a chip to change route order.
The lift and drop previews contain only the capsule.
The route action menu can undo the last route edit.

## Write and edit with Apple Pencil

Write in the entry field to add waypoints at its insertion point.
Write on a waypoint badge to edit that waypoint.
The editor places the cursor near the Pencil position.
It keeps the selection when the field already has focus.
The writing target stays with the field when the route layout changes.
Select Edit route text in the route action menu to edit the complete route.
This editor uses a native text view with Apple Pencil Scribble.
Scribble converts handwriting to text. It also supports selection and deletion gestures.
Write several identifiers with spaces between them.
Use text selection to replace or delete several identifiers together.
Press Return or select Add route to resolve the complete text.
Cancel edit keeps the installed route.
The editor does not change the installed route during a writing gesture.

Apple describes these interactions in
[Meet Scribble for iPad](https://developer.apple.com/videos/play/wwdc2020/10106/).

## Change an active route

1. Open Route Plan during local navigation.
2. Edit, insert, remove, or move a waypoint.
3. Inspect Current route and Updated route in the review.
4. Select Continue toward to identify the required navigation leg.
5. Select Apply update to use the proposed route.

The active route does not change while the review is open.
Cancel discards the proposal.
The current destination remains selected if its occurrence is in the proposal.
If that occurrence is absent, the reader must select a leg to continue navigation.
The default in that condition is Stop local navigation.

## Inspect an approach chart

1. Add the destination airport to the route.
2. Select Procedure charts.
3. Select a chart under On this route.
4. Check the chart edition and valid period.
5. Review the approach and missed-approach instructions.

The chart opens in a native PDF reader.
The reader can enlarge and move the chart.
The chart does not add executable procedure legs to the route.

## Weather-related route changes

Weather display and wind correction are deferred.
The editor does not assess weather or select a weather diversion.
The reader can use the active-route workflow after a separate weather review.
The selected vehicle profile remains attached to the route.
The Still air label remains visible when the estimate uses true airspeed.

Weather integration must use the same route proposal and review.
It must identify the observation or forecast time and the route arrival time.
It must show missing or stale coverage before the reader applies a change.
It must not replace an active route when a forecast refreshes.

## Regression checks

The simulator tests cover typed entry, Backspace, and a middle-waypoint replacement.
They enter successive waypoints and delete successive badges without additional taps.
They check multiple-identifier edits in the native text view.
They check inline field placement and unsupported-character rejection.
They check that an unresolved suffix does not apply a valid prefix.
They exercise nearby matching, amendment cancellation, amendment application, and leg progress.
They exercise native drag reordering and saved route order.
Unit tests check capsule preview bounds, input rules, wrapping, and route occurrence identity.
Unit tests also check amendment state and the route overview at the antimeridian.
Unit tests check text replacement through the native text input interface.
Physical Apple Pencil recognition requires an iPad and a paired Apple Pencil.
