# ADR-0043: Keep the GDL 90 codec in a crate of its own

- Status: Proposed
- Date: 2026-09-17

## Context

GDL 90 is the data interface between an ADS-B receiver appliance and a
display. An appliance sends a heartbeat, its own position and attitude, the
traffic that it sees, and the uplink data that it receives. A display reads
them. Both sides must agree byte for byte.

Pilotage is a display of that interface. The Apple client reads a GDL 90
stream from an AeroLink appliance and turns it into traffic observations of
the surveillance domain and into the navigation state of the ownship.

AeroLink is a receiver. It decodes Mode S and UAT from a radio, and an
AeroLink appliance encodes GDL 90 for a display. The GDL 90 encoders were
written in `aero-link-core`, next to the radio decoders.

That placement had two costs:

- A display took the codec from the receiver. Pilotage pinned an AeroLink
  revision to read GDL 90, and a change to the codec waited for an AeroLink
  release. The Apple client of Pilotage stopped at a pin that did not have
  the types that the client imported.
- The codec has no radio in it. It carries no Mode S, no UAT decoding and no
  USB. A microcontroller firmware, a desktop receiver and a tablet client each
  need it, and none of them needs the rest of the receiver for it.

## Decision

The GDL 90 codec is a crate of its own: `gdl90`, in the repository
`luofang34/Gdl90`.

The crate holds the wire format and nothing else:

- the frame with its CRC and byte transparency;
- the messages that an appliance sends: Heartbeat, Uplink Data, Ownship
  Report, Ownship Geometric Altitude, Traffic Report, the Stratux AHRS message
  and the ForeFlight extension AHRS message;
- one encoder and one decoder for each message.

The crate is `no_std`. It has no I/O, no clock, no allocation and no domain
model. An encoder writes into a buffer that the caller gives and leaves the
buffer unchanged on an error. An encoder refuses a value outside its range
and does not clamp it. A decoder gives a field that the message marks invalid
as `None`. A GPS track and an aircraft heading stay apart.

The dependency direction is:

```text
gdl90  <--  aero-link (appliance side: encodes)
gdl90  <--  pilotage-situation-ffi (display side: decodes)
```

Neither AeroLink nor Pilotage owns the codec, and neither depends on the other
for it.

The domain mapping stays outside the codec. The mapping of a Traffic Report
onto a surveillance observation belongs to the surveillance domain, as the
mapping of an AeroLink reception does today (`surveillance-aero-link`). The
mapping of an uplink onto weather products belongs to the weather domain.
The mapping of the ownship report and the attitude onto the navigation state
of the client belongs to the client. Until the domain repositories have these
adapters, the Apple binding of Pilotage holds the traffic mapping.

## Consequences

- The Apple client reads GDL 90 with the pinned AeroLink revision. The pin
  moves for receiver changes only.
- An appliance firmware and a display share one crate, so a codec defect is
  fixed in one place and each side gets it with a pin move.
- The codec has tests with the published examples of the specification, so a
  port to a new target starts from a known-good byte layout.
- Two follow-ups are open: `surveillance-gdl90` in the Surveillance
  repository for the traffic mapping, and `airmass-gdl90` in the Airmass
  repository for the uplink mapping. Each moves a mapping out of the Pilotage
  binding.
- AeroLink can drop its GDL 90 modules and take the crate. Its uplink encoder
  then gives the codec the corrected 432-byte payload, and the validity rules
  of a UAT uplink stay in AeroLink.

## Alternatives considered

- **Keep the codec in `aero-link-core`.** Rejected. A display then depends on
  a receiver for a format that has no radio in it, and each codec change
  waits for a receiver release.
- **Put the codec in a Pilotage crate.** Rejected. AeroLink cannot depend on
  Pilotage, so the appliance side would keep a copy, and the two copies would
  drift.
- **Put the codec in the Surveillance repository.** Rejected. GDL 90 carries
  weather uplinks and attitude too, and the appliance side would then depend
  on one domain for a format that spans three.
