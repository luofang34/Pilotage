# hosts/ — session-host binaries

The session host is the unit of deployment
([ADR-0004](../docs/adr/0004-host-oriented-topology.md)): it runs one or more
adapters, the embedded authority engine, and the session-local real-time
telemetry/control/media endpoint. The same binary serves server-hosted production,
loopback/LAN demos, and (later) peer-hosted operation.

Planned contents:

- `session-host/` — the main host binary: WebTransport (QUIC) endpoint, authority
  engine, adapter lifecycle, capability publication, structured event emission.
