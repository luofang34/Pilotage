# ADR-0016: The media plane is codec-pluggable; the control core never sees the codec

- Status: Accepted
- Date: 2026-07-05

## Context

The first Gazebo demo streams video as MJPEG: per-frame JPEG, encoded in software.
That is right for a proof of concept — trivial to encode, decode-anywhere (native
`image`, browser `createImageBitmap`), no inter-frame state. It does not survive
contact with real resolutions. Software encode runs out somewhere around 1080p60;
4K low-latency is hardware-only, and hardware encoders are platform-specific
(VideoToolbox on macOS, NVENC/VAAPI on Linux).

Platform-specific means the codec is platform code by ADR-0002's rule, so it cannot
live in the portable sans-IO core, and by ADR-0003 it belongs to the media plane,
which is already a separate responsibility from the real-time data plane
(control / telemetry / authority). The risk is not the codec choice — it is letting
a video or codec type leak into the control or authority messages, which would couple
the portable control model to a platform concern and make the eventual hardware swap
a protocol change across independently deployed hosts and clients (exactly what
ADR-0014 exists to avoid).

## Decision

- Video is a **codec-pluggable** concern of the media plane. The control, telemetry,
  and authority messages MUST remain codec-agnostic: no video or codec type appears
  in any non-media message.
- Each video frame on the wire carries a **FourCC codec tag** (32 bits, four ASCII
  bytes as written) in its header, distinct from the payload length and bytes:

  ```text
  video uni-stream: [0x02 stream-kind][fourcc: 4 bytes][u32 LE payload_len][payload]
  fourcc: "MJPG" MJPEG (only one implemented) · "avc1" H.264 · "hvc1" H.265 · "av01" AV1 · "vp09" VP9
  ```

  The client dispatches on the FourCC. Adding a codec is then a non-event: a new
  tag value, a new decoder branch, no change to the envelope, the control path, or
  the authority path.

  **Why FourCC and not a private byte.** There is no widely-accepted 1-byte codec
  enumeration; a private byte would interoperate with nothing and force us to run our
  own registry. FourCC is the de-facto convention for naming a codec in a fixed-width
  field, and the video-codec tags (`avc1`, `hvc1`, `av01`, `vp09`) are the ISO-BMFF
  sample-entry codes governed by MP4RA, so the value space is already arbitrated. It
  self-documents in packet captures and muxes directly into MP4/MKV. The three extra
  bytes per frame are negligible.

### Codec identity has two levels; RFC 6381 owns the second, not the first

- **Per-frame — FourCC (routing).** "Which decoder does this frame belong to." Fixed
  width, and it has a real value for the demo codec (`"MJPG"`). This is all a per-frame
  field should carry.
- **Per-stream — RFC 6381 codec string (configuration).** "How is that decoder
  configured" — profile, level, tier. It is variable-length and stream-constant, so
  it belongs in a **media-config message** sent once (reliably) when a video source is
  announced, referenced by frames via a source id. It is consumed verbatim by the
  browser's WebCodecs `VideoDecoder.configure({codec})`.

  RFC 6381 is **not** adopted for the PoC, deliberately: MJPEG has no established RFC
  6381 codec string (inventing one would be the same namespace-squatting a private
  byte would be), the PoC browser path decodes MJPEG via `createImageBitmap` rather
  than WebCodecs, and profile/level do not exist for MJPEG. RFC 6381 and the per-stream
  media-config message land together with the first parameterized codec
  (software H.264 + WebCodecs), where they first do real work. Until then the FourCC
  routing tag is sufficient on its own.
- Encoding is confined to **one isolated module** in the session host, so replacing
  the encoder is a local change.
- Hardware and additional software encoders are **media-plane platform ports** when
  they land (ADR-0002), never core code.

### The ladder (informative, not a commitment to build ahead)

1. MJPEG, software — the demo (this increment).
2. Software H.264 + browser WebCodecs decode — next, for bitrate and quality.
3. Hardware H.264 (VideoToolbox / NVENC / VAAPI) as platform ports — for 1080p60/4K.

Each rung is a drop-in behind the FourCC tag: the wire envelope and the control/
authority core are unchanged across all three.

## Consequences

- The FourCC tag is the one thing that would be expensive to retrofit (it spans
  independently deployed hosts and clients), so it ships **now**, with only MJPEG
  (`"MJPG"`) implemented.
- A `VideoEncoder` trait is deliberately **not** introduced yet: one implementation
  does not justify the abstraction. The codec id plus an isolated encode module is
  the right amount of seam; the trait earns its place when the second encoder
  actually lands.
- Decoders MUST treat an unknown FourCC as a skipped frame with a counted, logged
  warning — never a hard failure — so a newer host streaming a codec an older client
  lacks degrades gracefully.
