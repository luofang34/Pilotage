# ADR-0005: WebTransport as the primary real-time transport, including media

- Status: Accepted
- Date: 2026-07-05

## Context

The browser must receive low-latency rendered video and send control input, for
centrally hosted, local, and future peer-hosted session hosts. WebSocket's ordered
reliable delivery delays fresh control behind retransmissions, so a
head-of-line-blocking-free transport is required.

WebTransport (QUIC/HTTP-3) is Baseline across engines: Chrome 97+, Edge 98+,
Firefox 114+, and — since Safari 26.4 (March 2026) — macOS, iOS, and iPadOS,
including datagrams, unidirectional and bidirectional streams, and backpressure.
WebCodecs `VideoDecoder` has shipped in Safari since 16.4 and in Blink/Gecko for
years. The historical reason to adopt WebRTC (the only browser path with unreliable
delivery and a media pipeline) no longer holds.

## Decision

- One **WebTransport session** per browser↔host connection carries all real-time
  traffic. Session bootstrap (authentication, admission, connect URL + token) uses
  plain HTTPS; there is no SDP/ICE negotiation.
- Message classes (ADR-0011) map onto WebTransport mechanisms:

  | Class | Mechanism | Semantics |
  |---|---|---|
  | `control-fast` | Datagrams | Unordered, unreliable; superseded samples droppable |
  | `control-edges` | Reliable ordered stream | Button edges and one-shots; never silently dropped |
  | `telemetry-fast` | Datagrams | Unordered, best effort |
  | `authority-events` | Reliable ordered stream (dedicated) | Lease grants, handover, override, revocation, acknowledgements |
  | `bulk` | One stream per transfer | Profiles, capabilities, configuration, log fragments; no head-of-line contention with authority events |
  | `media` | Unidirectional stream per frame or GOP | Encoded video units with bounded lifetime; late units abandoned, not awaited |

- **Video** is encoded on the host (low-latency H.264 first; AV1/HEVC as adapters
  and hardware allow), carried as media-class streams, decoded in the browser with
  WebCodecs, and rendered to canvas/WebGPU. The client owns its jitter buffer and
  exposes per-stage timing (ADR-0009).
- **Bandwidth adaptation is ours to build**: delay- and loss-driven encoder rate
  control plus explicit keyframe-request messages. This is deliberate — an owned
  pipeline is instrumentable end to end, where WebRTC's congestion controller and
  jitter buffer are opaque.
- The host side is a pure-Rust QUIC/WebTransport endpoint (quinn-family stack),
  keeping the host free of SDP/ICE/DTLS-SRTP machinery.
- Message schemas remain transport-independent (ADR-0014): datagrams carry exactly
  one envelope-wrapped message; streams carry length-delimited envelopes.

### Browser floor

WebTransport-capable browsers: Safari 26.4+ (macOS/iOS/iPadOS), Chrome 97+,
Edge 98+, Firefox 114+. On iOS/iPadOS all browsers inherit WebKit, so the 26.4
system floor governs.

## Consequences

- No STUN/TURN/ICE planning; instead, the session host must be **dialable**: a
  reachable UDP/443 QUIC endpoint (with the platform-level HTTP/2 fallback where
  QUIC is blocked). For future peer-hosted topologies behind NAT, a QUIC relay
  (MASQUE-style) replaces WebRTC's hole-punching; hosted v1 is unaffected.
- TLS certificates are mandatory. Server-hosted v1 uses ordinary certificates. The
  local-demo strategy (locally provisioned dev certificate vs
  `serverCertificateHashes`, whose Safari support is unverified) is the open
  question tracked in ADR-0004.
- Datagram payloads must fit one QUIC packet (~1200 B MTU budget) — control frames
  are tens of bytes, so this constrains telemetry batching, not control.
- Media-pipeline work (WebCodecs decode, jitter buffer, rate control, keyframe
  recovery) is scheduled as an explicit spike in the first local-loop increment and
  validated under impaired networks in the hosted increment.
- Spectator fan-out later means host-side or relay-side stream replication rather
  than an SFU.

## Alternatives considered

- **WebRTC (media tracks + DataChannels):** the previous baseline while Safari
  lacked WebTransport. Mature congestion control and NAT traversal, but: two
  protocol stacks in one host (signaling + ICE + DTLS-SRTP alongside HTTPS), opaque
  jitter/CC behavior that fights the latency-instrumentation requirement, and a
  heavy host-side library choice (str0m vs webrtc-rs). Revisit trigger: field data
  showing NAT-blocked peer-hosted sessions that a relay cannot serve acceptably, or
  DIY rate control failing to converge under real network profiles.
- **WebSocket for everything:** head-of-line blocking puts retransmitted history
  ahead of fresh control samples; unacceptable for the control loop.
- **WebTransport for data + WebRTC for media only:** doubles the transport surface
  for one feature (built-in CC) that we intend to own anyway; worst of both worlds
  for a small team.
