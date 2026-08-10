# AirspaceView resolution contract

This contract defines how `AirspaceViewV1` adds horizontal geometry to an
aeronautical update.

## Owner

`pilotage-airspace-view` owns subject resolution.
The `AeronauticalUpdates` domain owns each update.
The Navdata domain owns each baseline snapshot.
The resolver owns no mutable state.
The resolver reads one immutable Navdata snapshot for one result.

## Snapshot identity

The result contains the Navdata cycle, snapshot ID, and snapshot digest.
The resolver verifies that the supplied cycle identifies the snapshot data.
An update subject also contains a Navdata cycle.
The resolver does not use an identifier with a different cycle.
The result keeps the subject cycle and stable identifier together.

## Geometry cases

An update can contain direct geometry.
The resolver uses direct geometry without a baseline lookup.

An update can name a baseline subject.
The resolver looks for that subject in the selected snapshot.
The result contains the stable subject identifier when a subject exists.

An update can have no useful horizontal geometry.
The resolver keeps this update in the result.
The item has no geometry.
The item contains a typed disposition.

## Typed failures

The result uses a typed reason for each failed lookup.
The reasons include these conditions:

- the identifier belongs to another cycle;
- the identifier is unknown;
- the identifier has more than one match;
- the snapshot does not carry the subject family;
- the snapshot carries the subject but not its geometry; and
- the snapshot cannot make the stated partial geometry.

A failed lookup does not remove the update.

## Partial subjects

The resolver does not enlarge a partial subject to a complete subject.
The `RunwaySegment` extent identifies a measured part of a runway.
The `FacilityComponent` extent identifies one facility component.
Direct partial geometry uses `GeometryCoverageV1::Partial`.
A direct geometry extent must equal the subject extent.
An extent mismatch returns `DirectGeometryExtentMismatch` and no geometry.
A baseline that cannot make the partial geometry returns
`PartialGeometryNotCarried`.

FAA NOTAM examples include a closure of the first 1,000 feet of one runway.
FAA guidance also identifies approach-light components as separate outages.
These records do not mean that the complete runway is closed.

Sources:

- [FAA NOTAM examples](https://www.faa.gov/air_traffic/publications/atpubs/notam_html/appendix_a.html)
- [FAA lighting aid NOTAM guidance](https://www.faa.gov/air_traffic/publications/atpubs/notam_html/chap5_section_2.html)

## Client rule

A map is supplemental.
It is not a complete view of aeronautical updates.
The client must show a list that contains every result item.
An empty map does not mean that no update applies.
