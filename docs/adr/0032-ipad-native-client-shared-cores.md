# ADR-0032: The Apple instrument composition boundary — Indicate owns the contract, the shells stay thin

- Status: Accepted
- Date: 2026-08-08 (revises the 2026-07-29 record)
- Tracking issue: [ARCH-INST-01](https://github.com/luofang34/Pilotage/issues/322)

## Context

The client must serve iPadOS natively. It must run EFB preflight with no
host. It must run the full control terminal against a host. One codebase
switches posture by discovery ([ADR-0026](0026-host-capability-profiles.md)).

The instrument family now lives in the Indicate repository
([ADR-0034](0034-extraction-boundary.md)). ADR-0035 assigns the ownership:
Indicate owns the instrument state contracts, the panel sets, the scene
contracts, the registry, and the admission tests. A Swift CoreGraphics scene
interpreter exists and passes the scene conformance corpus
([#255](https://github.com/luofang34/Pilotage/issues/255)). The boundary
between Pilotage, Indicate, and the Apple backend was not written down. This
record defines that boundary.

The earlier revision of this record used the names Communicate and
aerocontext. [ADR-0035](0035-source-neutral-situational-services.md) owns the
current names. The advisory domain is AeroContext. This record uses the
ADR-0035 names.

## Decision

### The contract and panel owner is Indicate

- Indicate owns the instrument state contract, the scene contract, the panel
  sets, the registry, and the admission tests.
- Pilotage consumes Indicate at one exact revision
  ([ADR-0034](0034-extraction-boundary.md)).
- Pilotage does not copy panel logic, glyphs, or contract vocabulary into a
  platform layer.

### The Apple display backend is IndicateAppleDisplay

- IndicateAppleDisplay is the Swift package that decodes scene bytes and
  paints them with CoreGraphics on Apple platforms.
- IndicateAppleDisplay owns display interpretation and failure latching on
  the Apple platform.
- IndicateAppleDisplay does not own panel logic. It does not own state
  derivation. It interprets the scene bytes that the shared runtime produces.

### Pilotage owns one portable instrument runtime

- Pilotage keeps one platform-neutral Rust crate: the Pilotage instrument
  runtime.
- The runtime owns these functions:
  - state decode and resolve;
  - feeder state;
  - alert step;
  - panel configuration;
  - scene generation and validation;
  - successful-production generation;
  - typed producer status.
- The runtime does not depend on `wasm_bindgen`, `serde_wasm_bindgen`, the
  Pilotage protocol, JavaScript, Swift, or a UI framework.
- The browser shell is a thin WASM adapter over the runtime. The Apple shell
  is a thin bridge over the same runtime. Both adapters marshal data. Neither
  adapter owns a decision.

### The Apple bridge is a narrow generated FFI

- The Apple bridge exposes a generated FFI surface (the uniffi precedent of
  the AeroContext crates).
- The surface enumerates panel descriptors and the screen-composition layout.
- The surface returns typed scene bytes, the frame, the successful-production
  generation, the typed producer status, and the digest identity.
- Swift code does not implement panel logic. Swift code does not derive
  state. The Swift layer owns windowing, input, credentials, lifecycle, and
  rendering surfaces only.

### The compatibility tuple gates paint

The compatibility tuple has six values:

1. the state ABI version;
2. the scene format version;
3. the corpus version and the corpus digest;
4. the registry scene digest;
5. the screen-composition digest;
6. the glyph-pack content hash.

- The runtime computes the tuple from the Indicate revision that it links.
- Each shell pins the same tuple and checks it before the first paint.
- Each shell also computes the glyph manifest hash before it builds the atlas.
- A mismatch stops the paint. The shell shows the failure. The shell does not
  paint instruments from a backend that it did not verify.
- Pilotage verifies the backend that it ships. Pilotage does not keep a
  global reverse-dependency registry for Indicate
  ([#255](https://github.com/luofang34/Pilotage/issues/255)).

### One core, thin shells

- The session, wire, ingress, authority-view, input, instrument-state, and
  mission vocabularies run as the same Rust crates everywhere. The browser
  drives them as wasm. The iPad links them as a static library behind the
  generated FFI. A Linux native station links them directly.
- Panels render from the scene IR on every platform. IndicateAppleDisplay
  joins the browser canvas interpreter and the reference rasterizer as a
  third backend under the same conformance corpus and layer contract
  ([ADR-0017](0017-instrument-display-runtime.md),
  [ADR-0029](0029-panel-layout-look-plugins.md)).
- Transport stays in Rust. The iPad speaks the ordinary session protocol
  through the same QUIC/WebTransport client stack compiled into the app.
  Platform sockets are the only boundary crossed.
- The EFB posture embeds AeroContext in-process per
  [ADR-0026](0026-host-capability-profiles.md): navdata sync and store,
  identifier resolution, route expansion, and briefings run on-device. The
  same snapshot surface that the host consumes feeds preflight with no host
  present ([ADR-0035](0035-source-neutral-situational-services.md)).
- A companion-computer build stays free of Swift and Apple dependencies
  ([ADR-0035](0035-source-neutral-situational-services.md)).

## Consequences

- The platform-specific surface of the Apple app is deliberately small:
  shell, input, credentials, rendering context, and a QUIC socket. Every
  decision lives in shared, tested code.
- The compatibility tuple makes a backend mismatch a stopped paint, not a
  wrong picture. Each consumer proves the backend that it ships.
- The portable runtime has one implementation for the WASM adapter and the
  Apple bridge. The conformance suite runs against both ports
  ([ADR-0002](0002-cargo-workspace-portable-sans-io-core.md)).
- WebTransport-over-QUIC from a native app avoids the certificate
  constraints of the browser. The host trust story
  ([ADR-0004](0004-host-oriented-topology.md)) still applies.
- Apple-platform release mechanics (signing, distribution, background
  execution limits) are deployment concerns. Implementation records them.

## Alternatives considered

- **SwiftUI-native instruments that reuse only the data model:** rejected.
  This forks panel behavior and assurance. The scene IR exists to prevent
  that fork.
- **A WKWebView wrapper around the web client:** rejected as the product
  path. It inherits the browser transport and certificate constraints. It
  makes hardware input and lifecycle second-class.
- **Platform networking with a Swift session layer:** rejected. It
  duplicates the wire and session state machines in a second language.
  ADR-0002 exists to prevent that cost.
- **A hand-written C header for the Apple bridge:** rejected. A generated
  FFI keeps the surface mechanically aligned with the Rust types. The
  AeroContext crates prove the generator on this toolchain.
