# H.264 Annex-B baseline fixture

A short, deterministic H.264 Annex-B elementary stream used to exercise the
browser decoder's Annex-B classification (keyframe detection, in-band SPS/PPS
presence, and `avc1.PPCCLL` config extraction) over real encoder output rather
than only synthetic NAL units. It is not decoded in CI (WebCodecs needs a
browser); `video-h264.test.mjs` runs the parser/keyframe/config path over it and
pins its digest.

- File: `h264-annexb-baseline.h264` (1640 bytes)
- SHA-256: `84d843b4334d9a5a2aec482d0a56f4fb60ce450a5c87b6f8414eb9d3a39fe6c7`
- Leading access unit NAL types: 7 (SPS), 8 (PPS), 6 (SEI), 5 (IDR), then 1 (non-IDR slices)
- Codec string: `avc1.42c00a` (baseline profile, level 1.0)

## Provenance (reproducible)

Encoder: ffmpeg version 8.1.2 Copyright (c) 2000-2026 the FFmpeg developers

Command:
```
ffmpeg -y -f lavfi -i "testsrc=size=48x32:rate=5:duration=1" \
  -c:v libx264 -profile:v baseline -pix_fmt yuv420p \
  -g 5 -x264-params "keyint=5:min-keyint=5:scenecut=0" \
  -f h264 h264-annexb-baseline.h264
```

The input is FFmpeg's synthetic `testsrc` pattern, so no external media is
involved. libx264's output for a fixed input and these parameters is
deterministic enough to serve as a pinned fixture; if a future libx264 changes
the bytes, regenerate and update the digest above.
