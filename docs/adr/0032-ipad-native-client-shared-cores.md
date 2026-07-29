# ADR-0032: The iPad client is a thin native shell over the same Rust cores the web client runs as wasm

- Status: Proposed
- Date: 2026-07-29

## Context

The client must serve iPadOS natively — EFB preflight with no host at
all, and the full control terminal against a host — as one codebase
posture-switched by discovery ([ADR-0026](0026-host-capability-profiles.md)).
The architecture already forbids the trap that makes native ports
expensive: no business rule or wire-level state machine may live only in
one platform layer ([ADR-0002](0002-cargo-workspace-portable-sans-io-core.md)),
panels are pure state→scene functions over a versioned scene-command IR
with two conformance-checked backends ([ADR-0017](0017-instrument-display-runtime.md),
[ADR-0029](0029-panel-layout-look-plugins.md)), and the instrument crate
family is `no_std`. Communicate's crates are proven
`aarch64-apple-ios`-clean with a uniffi FFI surface designed for exactly
this embedding. The known debt is the web client's JS-only feeder logic
(telemetry ingress gating, derivations), which [ADR-0029](0029-panel-layout-look-plugins.md)
already commits to moving behind the shared core boundary.

This record sets the design envelope; implementation is deferred.

## Decision

- **One core, three shells.** The session, wire, ingress, authority-view,
  input, instrument-state, and mission/plan vocabularies run as the same
  Rust crates everywhere: the browser drives them as wasm, the iPad links
  them as a static library behind a generated FFI (the aerocontext uniffi
  precedent), and any Linux native station links them directly. The Swift
  layer owns only: windowing/scenes, touch and hardware input, keychain
  credentials, lifecycle, and rendering surfaces.
- **Panels render from the scene IR, natively.** A Metal/CoreGraphics
  scene-command interpreter joins the browser canvas interpreter and the
  reference rasterizer as a third backend under the same conformance
  corpus and layer contract. Panel code, glyphs, and budgets are reused
  byte-identically; no Swift redraws an instrument.
- **Transport stays in Rust.** The iPad speaks the ordinary session
  protocol through the same QUIC/WebTransport client stack compiled into
  the app, driven by the sans-IO session core — not through a parallel
  platform networking implementation. Platform sockets are the only
  boundary crossed.
- **EFB posture embeds Communicate in-process** per
  [ADR-0026](0026-host-capability-profiles.md): navdata sync/store,
  identifier resolution, route expansion, and briefings run on-device
  through Communicate's device-clean crates; the same snapshot surface
  the host consumes ([ADR-0030](0030-communicate-navdata-provisioning.md))
  feeds preflight with no host present.
- **Porting debt is named, not discovered.** Before the iPad shell
  starts, the JS-only feeder logic moves behind the shared boundary
  ([ADR-0029](0029-panel-layout-look-plugins.md)): telemetry ingress
  gating, turn/heading derivations, and the display-profile scaling
  introduced by [ADR-0031](0031-nav-guidance-telemetry-display.md). The
  wasm and FFI builds then drive one implementation, and the conformance
  suite runs against both ports ([ADR-0002](0002-cargo-workspace-portable-sans-io-core.md)).

## Consequences

- The iPad app's platform-specific surface is deliberately boring: shell,
  input, credentials, rendering context, and a QUIC socket — everything
  that decides anything is shared and already tested.
- WebTransport-over-QUIC from a native app avoids the browser's
  certificate constraints; the host trust story (certificate strategy,
  [ADR-0004](0004-host-oriented-topology.md)) still applies and is not
  weakened by the native path.
- The scene-IR native backend is the main new build; the conformance
  corpus makes its correctness a mechanical comparison rather than a
  visual judgment.
- Apple-platform release mechanics (signing, App Store or ad-hoc
  distribution, background execution limits for sync) are deployment
  concerns recorded when implementation starts, not architecture.

## Alternatives considered

- **SwiftUI-native instruments reusing only the data model:** rejected;
  it forks panel behavior and assurance, exactly what the scene IR
  exists to prevent.
- **Wrapping the web client in a WKWebView:** rejected as the product
  path (kept as a debugging convenience); it inherits browser transport
  and certificate constraints, cannot embed Communicate natively, and
  makes hardware input and lifecycle second-class.
- **Platform networking (Network.framework QUIC) with a Swift session
  layer:** rejected; it duplicates the wire/session state machines in a
  second language, the one cost ADR-0002 exists to prevent.
