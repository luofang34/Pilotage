# ADR-0045: The host is a library that a client can embed

- Status: Proposed
- Date: 2026-09-23

## Context

Pilotage runs in three shapes:

1. An onboard host with no display. It flies missions and serves agents.
2. An operator client, such as an iPad or a Vision Pro, that connects to a
   host on another device.
3. An electronic flight bag (EFB) on an iPad with no external host. It plans,
   briefs, and shows the situation from its own data sources.

ADR-0037 lets a client compose a local situation view when no host exists. It
rejected a separate local host process. The result is two composition paths:
one in the host, and one in the client. Both assemble the same domains
(ADR-0036). They can drift apart, and each new domain must be added twice.

## Decision

### One composition, two transports

The host composition is a library, `pilotage-host-core`. It contains the
domain owners, the `SituationView`, the authority engine, and the session
services. It has no network, window, or process code.

- An **onboard host** links the library into the session-host binary and
  serves clients over WebTransport (ADR-0005).
- An **EFB with no external host** links the same library into the app. The
  client modules connect to it through an in-process transport that carries
  the same message classes (ADR-0011) without serialization to a network.
- An **operator client with an external host** does not start the library.
  It connects over WebTransport.

A client module cannot tell which transport it uses. The situation assembly
exists once.

### Local sources join through the same adapters

An EFB's local sources, such as a GDL 90 receiver, downloaded weather, and
installed packs (ADR-0044), are host adapters in the embedded library. They
are not client code. When the EFB later connects to an onboard host, the
same adapters can run there.

### What ADR-0037 keeps

ADR-0037 rejected a local host process. This decision does not add a
process. The embedded library runs on the app's own threads. The client
module selection of ADR-0037 does not change.

### Performance

- The in-process transport passes messages by value in memory. It does not
  encode protobuf, and it does not copy large payloads more than once.
- Media frames in the embedded case pass by shared buffer handle.
- The embedded library runs its domain work on background queues, so the
  display thread does not wait for it.

## Consequences

- Each domain is composed once. The iPad EFB and the onboard host show the
  same situation from the same inputs.
- The embedded library must build for iPadOS, visionOS, and WebAssembly. Its
  crates must stay sans-IO at their core (ADR-0002), with platform adapters
  at the edges.
- A client that moves from standalone to connected changes only its
  transport. Its modules and its data do not move.
- Migration: the local situation composition in the Apple client moves into
  host adapters. This migration is tracked separately.
