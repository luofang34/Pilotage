# ADR-0002: One Cargo workspace with a portable sans-IO core

- Status: Accepted
- Date: 2026-07-05

## Context

The v1 deliverable is a browser client (partly compiled to WebAssembly) controlling a
server-hosted Gazebo simulation. Native operator stations are a required future
target, and hosts, adapters, services, and conformance tooling are all Rust. Two risks
pull in opposite directions:

- If domain logic leaks into browser-specific code, native clients later require a
  second implementation of protocol, input, authority, and timing behavior.
- If browser and native are forced through one lowest-common-denominator platform
  abstraction, their genuinely different device, credential, and media APIs get
  obscured rather than isolated.

A further constraint: identical behavior must be *provable* across browser, native,
and test targets. An authority or timing bug reproducible only inside a wasm build in
a live transport session would be nearly undiagnosable.

## Decision

1. The repository is a single Cargo workspace (monorepo) containing core crates,
   hosts, adapters, clients, and services.
2. Domain logic lives in **portable sans-IO core crates** under `crates/`:
   - They contain pure state machines and data types. Inputs are messages, events,
     and explicit `now` timestamps; outputs are messages, actions, and requested
     timer deadlines.
   - They MUST NOT depend on tokio, wasm-bindgen, web-sys, sockets, threads, system
     clocks, or engine SDKs. Nondeterminism is what is excluded, not the standard
     library.
3. Platform ports drive the state machines and own all I/O:
   - Browser ports: Gamepad, WebAuthn, WebTransport, WebCodecs, Web Workers,
     rendering (wasm/JS/TS).
   - Native ports (future): HID/SDL/gilrs, OS credential APIs, native QUIC,
     hardware codecs, windowing.
   - Host runtime: tokio, network listeners, adapter process management.
4. No business rule or wire-level state machine may exist solely in JavaScript,
   TypeScript, or any single platform layer.
5. Core crates materialize on demand rather than all up front. The seed set:

   | Crate | Responsibility |
   |---|---|
   | `pilotage-protocol` | Wire types generated from `schemas/`, envelopes, version negotiation |
   | `pilotage-authority` | Lease, generation, handover, override, link-loss state machines |
   | `pilotage-input` | Canonical input model, device profiles, normalization pipeline |
   | `pilotage-timing` | Time model, latency accounting, staleness policy |
   | `pilotage-adapter-api` | Adapter traits and capability model |
   | `pilotage-conformance` | Shared fixtures and behavioral test suites |

   Recording/replay, telemetry modelling, and transport-session logic split into
   their own crates when they exceed the size limits in ADR-0015, not before.

## Consequences

- Every core behavior is testable with `cargo test` on a development machine — no
  browser, simulator, or network required.
- Deterministic replay (ADR-0012) falls out of the sans-IO discipline: replaying
  recorded inputs and timestamps through the same state machines *is* the test.
- Browser and native clients MUST pass the same conformance suite through their
  respective ports.
- Thin platform-specific code is accepted; duplicated domain logic is not.
- Core crates must not grow `async fn`, clock reads, or I/O; CI enforces the
  dependency bans (ADR-0015).

## Alternatives considered

- **Multiple repositories per plane:** rejected; protocol, host, and client evolve in
  lockstep during early development, and cross-repo atomic changes are costly.
- **Async-first core (tokio everywhere):** rejected; it would force a runtime into
  wasm builds, entangle timing with the scheduler, and make deterministic replay and
  conformance testing far harder.
