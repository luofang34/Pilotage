# X-Plane mission planner check

## Test configuration

The test uses X-Plane 12.4.3-r2 and the C172 G1000 aircraft.
Pilotage runs on a dedicated iPad simulator with iOS 27.
The test route is `KEWR ARD ZUBAX COPIX KTTN`.
The saved vehicle profile is `C172 G1000 Simulator` with 110 KTAS.
KTAS means knots of true airspeed.

The navigation data and approach chart use FAA cycle 2609.
The selected chart is `ILS OR LOC RWY 06` at KTTN.
Its valid period is 3 September through 1 October 2026.
The test enters the route and vehicle profile through the Pilotage interface.
The chart opens from the route planner.

The test controller uses the local X-Plane web API.
It changes cockpit controls and autopilot commands with aircraft physics enabled.
Pilotage does not send aircraft control commands in this test.
The PDF chart does not add executable procedure legs or a hold to the route.

## Changes from the workflow check

- Scribble targets the entry field and each waypoint badge separately.
  A new writing gesture uses the position in that text field.
  A field with focus keeps its text selection.
- The complete-route text editor remains an explicit route action.
  It supports selection and replacement of several identifiers.
- The planner opens installed procedure charts for airports on the route.
  The native PDF reader uses a large page presentation.
- Local package imports use one file copy and one final disk synchronization.
  Checksum verification and atomic installation remain required.
- Starting local navigation requests a position when no position is available.
- Follow combines position, scale, and heading in one camera animation.
  Its first scale change waits for a position if permission is still required.
  Later position updates do not change the selected scale.

## Verification

| Check | Result |
| --- | --- |
| Route input unit tests | 14 passed, including selection, wrapped entry coordinates, and stable writing targets |
| Route leg tests | 3 passed |
| Map and control tests | 9 passed, including combined camera commands and a delayed first position |
| Route editing interface tests | 4 passed |
| Fresh-install route and approach-chart interface test | Passed in 27 seconds |
| Aviation data tests | 13 passed |
| Package store tests | 18 passed |
| Package format, Clippy, documentation, and release build | Passed |
| Forbidden Rust filenames | Passed |

One combined unit and interface run failed during initial data installation.
The route did not resolve within the test timeout.
A separate interface test on a fresh installation passed.
The failure remains recorded in the test results.

The physical iPad build and installation passed.
The user unlocked the device, and the app opened on the iPad.
The test entered `KEWR ARD`, pressed Return, and entered `ZUBAX COPIX KTTN`.
The entry field kept focus between the two groups.
Two Backspaces removed KTTN and COPIX in sequence.
Typing `COPIX KTTN` restored both waypoints at the end of the route.
Native text input and Scribble delegate tests passed on the simulator.
Physical handwriting recognition has not been verified.

## Flight record

The aircraft departed KEWR runway 29 and flew toward ARD.
The approach check crossed ARD and reached ZUBAX at approximately 2,700 feet.
Direct heading-value writes caused an incorrect turn direction during the first course reversal.
The test uses native heading-knob commands to change the heading target.
The next course reversal used the required left turns.
The localizer captured near ZUBAX.
The glideslope captured at approximately 1,900 feet before COPIX.
The aircraft followed the ILS to runway 6 and landed.
It completed rollout at less than 3 metres per second.
The simulator then paused.

The recorded touchdown position is approximately 396 metres beyond the runway threshold.
It is approximately 4 metres from the runway centreline.
The runway width is 46 metres in the installed X-Plane airport data.
The touchdown was firm.
The indicated descent rate at touchdown was approximately 640 feet per minute.
X-Plane reported no crash and no collapsed landing gear.
This result does not verify the landing controller for general use.

Initial simulator attempts included random system failures and incorrect altitude-mode commands.
Those attempts do not establish a successful approach.
The test disables random failures and checks vertical-speed and altitude-arm states.

The user authorized Allow Once on the mission test simulator.
The test sent recorded X-Plane positions through the simulator location service.
The ARD crossing changed the current destination to ZUBAX.
The ZUBAX crossing changed it to COPIX.
The COPIX crossing changed it to KTTN.
The NavLog and map showed the corresponding past, current, and planned legs.
This check used flight position replay, not a simultaneous live flight connection.

The test also read the current position from the paused X-Plane instance.
It sent that position directly to the mission test simulator.
The final app build centred the map near KTTN and set the initial scale after Allow Once.
The aircraft remained at KTTN after rollout.
The position was approximately 475 metres from the KTTN airport reference point.
The route arrival limit is 185.2 metres from the waypoint.
Thus, this landing position did not complete the route automatically.

The session-host instrument feed carries local north/east/down coordinates.
Its telemetry schema does not supply a geographic position or geographic origin.
Those coordinates cannot identify a position on the map.
The map receives aircraft geographic positions through the AeroLink receiver path.
The current app has no UDP receiver for a direct X-Plane position feed.

X-Plane displayed a low-frame-rate warning during the flight.
The test uses simulator time for the timed course-reversal leg.

Local test artifacts are in `clients/apple/.build/mission-qa/`.
They include the saved mission, flight traces, test logs, and chart screenshot.
`scribble-target-tests.log` records the final 14 unit tests and 4 interface tests.
`flight-result.json` records the approach capture and runway positions.
`route-crossing-samples.json` records the positions used for the leg replay.
`live-position-check.json` records the direct position read from paused X-Plane.
`follow-camera-tests.log` records the camera and control regression tests.
`physical-follow-build.log` records the final physical-device build.
`kttn-rollout.png` shows the aircraft on runway 6 after rollout.
The saved mission ID is `3954E55B-609B-4DE9-9DBB-487B6DF32A81`.
