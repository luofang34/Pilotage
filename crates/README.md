# crates/ — portable sans-IO core

Domain logic shared by every client, host, and tool. Rules
([ADR-0002](../docs/adr/0002-cargo-workspace-portable-sans-io-core.md)):

- Pure state machines and data types: inputs are messages, events, and explicit
  `now` timestamps; outputs are messages, actions, and requested timer deadlines.
- No tokio, wasm-bindgen, web-sys, sockets, threads, system clocks, or engine SDKs.
- Everything here must be exercisable by `cargo test` alone.

Seed crates (created as implementation starts, split further per the size limits in
[ADR-0015](../docs/adr/0015-workspace-quality-gates.md)):

| Crate | Responsibility |
|---|---|
| `pilotage-protocol` | Wire types generated from `schemas/`, envelopes, version negotiation |
| `pilotage-authority` | Lease, generation, handover, override, link-loss state machines |
| `pilotage-input` | Canonical input model, device profiles, normalization pipeline |
| `pilotage-timing` | Time model, latency accounting, staleness policy |
| `pilotage-adapter-api` | Adapter traits and capability model |
| `pilotage-conformance` | Shared fixtures and behavioral test suites |
