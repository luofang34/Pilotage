# services/ — optional central services

Central services support sessions but must never be required for the real-time data
path or for session existence
([ADR-0003](../docs/adr/0003-separate-responsibility-planes.md),
[ADR-0004](../docs/adr/0004-host-oriented-topology.md)).

Planned contents:

- `identity-admission/` — passkey/WebAuthn authentication, membership, session
  capabilities, signaling/rendezvous for v1.

Fleet orchestration, organizational policy services, and QUIC-relay provisioning join
later as separate deployables; none of them enter the session data-plane correctness
boundary.
