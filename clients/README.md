# clients/ — operator front ends

Thin platform layers over the portable core
([ADR-0002](../docs/adr/0002-cargo-workspace-portable-sans-io-core.md)). No business
rule or wire-level state machine may live only here.

Contents:

- `apple-instrument-bridge/` — thin generated FFI bridge (uniffi) to the
  instrument runtime for Apple shells ([ADR-0032](../docs/adr/0032-ipad-native-client-shared-cores.md)).
  Companion-computer builds do not link it.
- `apple-instrument-consumer/` — thin Swift compatibility gate and scene
  producer for IndicateAppleDisplay. It does not own panel logic.

Planned contents:

- `web/` — v1 browser client: wasm build of the core plus browser ports (Gamepad,
  WebAuthn, WebTransport, WebCodecs, workers, rendering) and the UI application.
- `native/` — future native operator station: platform HID, OS credentials, native
  transport and codecs. Required architectural target; created when work begins.

Both clients must pass the same conformance suite through their ports.
